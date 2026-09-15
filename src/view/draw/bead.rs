//! A bead's own line, and the run of closed siblings drawn in place of the
//! several beads it stands for.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::config::Colour;
use crate::model::badges::Badged;
use crate::model::types::Status;
use crate::view::fitted::{columns, openable, Block, Fitted, Link, Shorter, GAP};
use crate::view::palette;
use crate::view::phrase;
use crate::view::row::{self, Cell, Layout, Row, Widths, AGENT, WARNING};

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
/// `widths` is how wide each identity cell is drawn on the widest line of
/// the forest, so a column of them lines up under one another and the titles
/// start together.
pub(super) fn bead_line(row: &Row, prefix: &str, widths: &Widths, layout: &Layout) -> Fitted {
    let walk = Walk {
        row,
        layout,
        widths,
    };
    let identity = walk.block(
        Block::Identity,
        vec![structure(prefix)],
        row.agent.as_deref(),
    );
    let title = walk.block(Block::Title, Vec::new(), row.agent.as_deref());
    let state = walk.block(Block::State, Vec::new(), row.agent.as_deref());

    let mut links = identity.links;
    links.extend(title.links);
    links.extend(state.links);
    let mut shorter = identity.shorter;
    shorter.extend(title.shorter);
    shorter.extend(state.shorter);

    let mut fitted = Fitted::new(identity.spans, title.spans, state.spans)
        .linking(links)
        .shortening(shorter);
    if layout.state.contains(&Cell::Agent) {
        if let Some(briefly) = &row.agent_briefly {
            let short = walk.block(Block::State, Vec::new(), Some(briefly));
            fitted = fitted.briefly(short.spans, short.links, short.shorter);
        }
    }
    fitted.toned(tone(row))
}

/// How wide each identity cell of `layout` draws on this row, in columns as
/// `fitted` measures a span: what the forest's width table is the widest of.
pub(crate) fn identity_widths(row: &Row, layout: &Layout) -> Widths {
    let walk = Walk {
        row,
        layout,
        widths: &Widths::default(),
    };
    layout
        .identity
        .iter()
        .map(|cell| (cell.clone(), walk.width(cell)))
        .collect()
}

/// One row's cells, drawn into whichever block the layout puts each in.
struct Walk<'a> {
    row: &'a Row,
    layout: &'a Layout,
    widths: &'a Widths,
}

/// One block of a row as the walk filled it: its spans, and which of them
/// are links or have short forms.
struct Walked {
    block: Block,
    /// The columns between one cell and the next: one in the identity, `GAP`
    /// in the other two blocks.
    apart: usize,
    spans: Vec<Span<'static>>,
    /// How many cells the block holds, which is not how many spans: the
    /// separators are spans, and the identity opens on a head that is not a
    /// cell.
    cells: usize,
    links: Vec<Link>,
    shorter: Vec<Shorter>,
}

impl Walk<'_> {
    /// `block` as the layout names it, on the `head` it opens with, with the
    /// agent said as `agent` where the block names one.
    fn block(&self, block: Block, head: Vec<Span<'static>>, agent: Option<&str>) -> Walked {
        let mut walked = Walked::opening(block, head);
        for cell in self.layout.block(block) {
            if block == Block::Identity {
                self.padded(&mut walked, cell, agent);
            } else {
                self.cell(&mut walked, cell, agent);
            }
        }
        if block == Block::State {
            self.trailing(&mut walked.spans);
        }
        walked
    }

    /// One cell of the identity, drawn through the columns the widest line
    /// draws it in: a cell this row says less in is padded out, and one it
    /// says nothing in still takes its column where any line draws it.
    fn padded(&self, walked: &mut Walked, cell: &Cell, agent: Option<&str>) {
        let width = self.widths.of(cell);
        let (from, cells) = (walked.spans.len(), walked.cells);
        self.cell(walked, cell, agent);
        if walked.spans.len() == from {
            if width > 0 {
                walked.cell(Span::raw(" ".repeat(width)));
            }
            return;
        }
        let apart = if cells > 0 { walked.apart } else { 0 };
        let drawn = columns(&walked.spans[from..]) - apart;
        if width > drawn {
            walked.spans.push(Span::raw(" ".repeat(width - drawn)));
        }
    }

    /// How many columns `cell` draws on this row, on its own.
    fn width(&self, cell: &Cell) -> usize {
        let mut walked = Walked::opening(Block::Identity, Vec::new());
        self.cell(&mut walked, cell, self.row.agent.as_deref());
        columns(&walked.spans)
    }

    fn cell(&self, walked: &mut Walked, cell: &Cell, agent: Option<&str>) {
        let row = self.row;
        match cell {
            Cell::Glyph => {
                walked.cell(Span::styled(
                    row.glyph.to_string(),
                    status_style(&row.status),
                ));
            }
            Cell::Id => {
                walked.cell(Span::styled(row.id.clone(), status_style(&row.status)));
            }
            Cell::Title => {
                walked.cell(Span::raw(row.title.clone()));
            }
            Cell::Badges => {
                for badge in row
                    .badges
                    .iter()
                    .filter(|badge| !self.layout.names(&badge.key))
                {
                    walked.badge(badge, &row.status);
                }
            }
            Cell::Badge(key) => {
                if let Some(badge) = row.badges.iter().find(|badge| badge.key == *key) {
                    walked.badge(badge, &row.status);
                }
            }
            Cell::Progress => {
                if let Some(progress) = row.progress {
                    walked.cell(Span::raw(done(progress.closed, progress.total)));
                }
            }
            Cell::Agent => {
                if let Some(agent) = agent {
                    let at = walked.cell(Span::styled(agent.to_string(), palette::AGENT));
                    // In the state the short form is the whole block's
                    // `briefly`; anywhere else it is this span's own.
                    if walked.block != Block::State {
                        if let Some(said) = &row.agent_briefly {
                            walked.shorter.push(Shorter {
                                block: walked.block,
                                at,
                                said: said.clone(),
                            });
                        }
                    }
                }
            }
            Cell::Anomalies => {
                if let Some(anomalies) = &row.anomalies {
                    walked.cell(Span::styled(anomalies.clone(), palette::ATTENTION));
                }
            }
        }
    }

    /// What the row reports about itself, after whatever the layout put in
    /// the state: the fold's counts, then the notes.
    ///
    /// After the row's own cells, because those name one bead and the counts
    /// count several: a number met before the name it belongs beside reads
    /// as the total the name is an example of.
    fn trailing(&self, state: &mut Vec<Span<'static>>) {
        let row = self.row;
        if let Some(shut_over) = &row.shut_over {
            if shut_over.live_agents > 0 {
                beside(
                    state,
                    Span::styled(
                        format!("{AGENT} {}", phrase::agents_beneath(shut_over.live_agents)),
                        palette::AGENT,
                    ),
                );
            }
            if shut_over.anomalies > 0 {
                beside(
                    state,
                    Span::styled(
                        format!(
                            "{WARNING} {}",
                            phrase::anomalies_beneath(shut_over.anomalies)
                        ),
                        palette::ATTENTION,
                    ),
                );
            }
        }
        for note in &row.notes {
            beside(state, Span::styled(note.clone(), palette::ATTENTION));
        }
    }
}

impl Walked {
    /// `block` with nothing in it yet but the `head` it opens on.
    fn opening(block: Block, head: Vec<Span<'static>>) -> Self {
        Walked {
            block,
            apart: if block == Block::Identity { 1 } else { GAP },
            spans: head,
            cells: 0,
            links: Vec::new(),
            shorter: Vec::new(),
        }
    }

    /// One more cell, `apart` columns after the one before it, and which
    /// span of the block it is.
    fn cell(&mut self, span: Span<'static>) -> usize {
        if self.cells > 0 {
            self.spans.push(Span::raw(" ".repeat(self.apart)));
        }
        self.spans.push(span);
        self.cells += 1;
        self.spans.len() - 1
    }

    /// A badge is a cell that is a link where its config gave it somewhere
    /// and has a short form where its config named one.
    fn badge(&mut self, badge: &Badged, status: &Status) {
        let at = self.cell(Span::styled(badge.text.clone(), badge_style(badge, status)));
        if let Some(to) = opens_at(badge) {
            self.links.push(Link {
                block: self.block,
                at,
                to: to.to_string(),
            });
        }
        if let Some(said) = badge
            .short
            .as_deref()
            .filter(|said| row::says_the_same_about_its_link(badge, said))
        {
            self.shorter.push(Shorter {
                block: self.block,
                at,
                said: said.to_string(),
            });
        }
    }
}

/// Where the badge takes a reader, where the emitter will write a sequence
/// saying so.
///
/// Both the underline and the hyperlink are asked of this one answer. The
/// underline is the whole of what a reader can *see* about a link, its
/// destination being nowhere in the row's text at any width, so a badge
/// marked as a link that does not open is a lie and one that opens unmarked
/// is never found.
///
/// Asked of the badge's own text rather than of the words the row goes on to
/// draw, so the answer does not turn on how wide the row is. A badge is a
/// link or it is not.
fn opens_at(badge: &Badged) -> Option<&str> {
    badge.link.as_deref().filter(|to| openable(&badge.text, to))
}

/// A badge's colour and its underline compose, because `palette::LINK` is an
/// underline carrying no colour of its own. A badge that names neither is
/// left with a style of nothing, which is what lets the row's own tone reach
/// it the way it reaches the title beside it.
fn badge_style(badge: &Badged, status: &Status) -> Style {
    let coloured = match badge.colour {
        Some(Colour::Status) => status_style(status),
        Some(Colour::Slot(slot)) => palette::slot(slot),
        Some(Colour::Absolute(colour)) => palette::absolute(colour),
        None => Style::new(),
    };
    if opens_at(badge).is_some() {
        coloured.patch(palette::LINK)
    } else {
        coloured
    }
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
            Painted::of(
                bead_line(&row(&node), BRANCH, &ids(4), &Layout::default()),
                46,
                1
            )
            .rows(),
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

        let drawn =
            Painted::of(bead_line(&epic, BRANCH, &ids(3), &Layout::default()), 60, 1).rows();

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

        let drawn =
            Painted::of(bead_line(&epic, BRANCH, &ids(3), &Layout::default()), 60, 1).rows();

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

        let drawn = Painted::of(
            bead_line(&shut, BRANCH, &ids(3), &Layout::default()),
            110,
            1,
        )
        .rows();

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

        let drawn = Painted::of(
            bead_line(&shut, BRANCH, &ids(3), &Layout::default()),
            110,
            1,
        )
        .rows();

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

        let drawn = Painted::of(
            bead_line(&shut, BRANCH, &ids(3), &Layout::default()),
            110,
            1,
        )
        .rows();

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

        let painted = Painted::of(
            bead_line(&shut, BRANCH, &ids(3), &Layout::default()),
            120,
            1,
        )
        .row(0);
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

        let wide = Painted::of(
            bead_line(&shut, BRANCH, &ids(3), &Layout::default()),
            120,
            1,
        )
        .rows();
        let narrow =
            Painted::of(bead_line(&shut, BRANCH, &ids(3), &Layout::default()), 68, 1).rows();

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

        let drawn =
            Painted::of(bead_line(&leaf, BRANCH, &ids(4), &Layout::default()), 60, 1).rows();

        assert!(!drawn[0].contains('/'), "{drawn:?}");
    }

    /// Ids are padded to the widest in the tree so the titles start together;
    /// a column that did not line up would be read as a tree shape it is not.
    #[test]
    fn ids_are_padded_so_the_titles_below_one_another_start_together() {
        let short = node("smt-4kd3p.1", "wire the niri theme include", Status::Open);
        let long = node("smt-4kd3p.20", "wallpaper timer calls dms", Status::Open);

        let short = Painted::of(
            bead_line(&row(&short), BRANCH, &ids(4), &Layout::default()),
            60,
            1,
        )
        .rows();
        let long = Painted::of(
            bead_line(&row(&long), BRANCH, &ids(4), &Layout::default()),
            60,
            1,
        )
        .rows();

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
        let drawn = Painted::of(
            bead_line(&row(&staffed), LAST, &ids(4), &Layout::default()),
            100,
            1,
        )
        .rows();

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

        let drawn = Painted::of(
            bead_line(&row(&staffed), LAST, &ids(4), &Layout::default()),
            50,
            1,
        )
        .rows();

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

        let drawn = Painted::of(
            bead_line(&row(&staffed), LAST, &ids(4), &Layout::default()),
            80,
            1,
        )
        .rows();

        assert!(drawn[0].contains("wallpaper timer calls dms"), "{drawn:?}");
        assert!(drawn[0].contains("wCM:p9 · working"), "{drawn:?}");
        assert!(!drawn[0].contains("elided run"), "{drawn:?}");
    }

    /// A badge the layout puts in the state beside the agent is still a link
    /// on the row that says the agent briefly. The short form is a state
    /// block of its own, and the badge is in it with everywhere it points.
    #[test]
    fn a_badge_in_the_state_still_opens_where_the_caption_gave_way() {
        let somewhere = "https://forge.invalid/orbital/atlas/pull/12";
        let mut staffed = captioned("teach the elided run to fold back open on a keypress");
        staffed.badges = vec![Badged {
            key: "delivery_pr".into(),
            text: "⇢ #12".into(),
            link: Some(somewhere.into()),
            short: None,
            colour: None,
        }];
        let layout = Layout {
            state: vec![Cell::Badge("delivery_pr".into()), Cell::Agent],
            ..Layout::default()
        };

        let drawn = Painted::of(bead_line(&row(&staffed), LAST, &ids(4), &layout), 80, 1).rows();
        let said = symbols(bead_line(&row(&staffed), LAST, &ids(4), &layout), 80);

        assert!(drawn[0].contains("⇢ #12  ◍ wCM:p9 · working"), "{drawn:?}");
        assert!(!drawn[0].contains("elided run"), "{drawn:?}");
        assert!(
            said.contains(
                &hyperlink("⇢ #12", somewhere).expect("this vocabulary holds no control character")
            ),
            "the badge lost its link when the agent was said briefly: {said:?}"
        );
    }

    /// And it still shortens there. 70 columns is a width the short form of
    /// the state fits at with its badge said shortly, and not with the badge
    /// said in full.
    #[test]
    fn a_badge_in_the_state_still_shortens_where_the_caption_gave_way() {
        let mut staffed = captioned("teach the elided run to fold back open on a keypress");
        staffed.badges = vec![shortenable(None)];
        let layout = Layout {
            state: vec![Cell::Badge("delivery_pr".into()), Cell::Agent],
            ..Layout::default()
        };

        let drawn = Painted::of(bead_line(&row(&staffed), LAST, &ids(4), &layout), 70, 1).rows();

        assert!(drawn[0].contains("wallpaper timer calls dms"), "{drawn:?}");
        assert!(drawn[0].contains("⇢ #12  ◍ wCM:p9 · working"), "{drawn:?}");
        assert!(!drawn[0].contains("atlas"), "{drawn:?}");
    }

    /// And it gives way only where it costs the title something. A row wide
    /// enough for both says what the pane says it is doing, which is the
    /// whole reason the caption is read off herdr at all.
    #[test]
    fn a_caption_the_title_does_not_need_the_room_for_is_said_in_full() {
        let staffed = captioned("teach the elided run to fold back open");

        let drawn = Painted::of(
            bead_line(&row(&staffed), LAST, &ids(4), &Layout::default()),
            120,
            1,
        )
        .rows();

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

        let drawn = Painted::of(
            bead_line(&row(&staffed), LAST, &ids(4), &Layout::default()),
            57,
            1,
        )
        .rows();

        assert!(drawn[0].contains("wallpaper timer calls dms"), "{drawn:?}");
        assert!(drawn[0].contains("◍ dish · working"), "{drawn:?}");
    }

    /// The bead is still named by its id, which is fitted before either of
    /// them and cannot be crowded out by anything.
    #[test]
    fn nothing_on_the_row_can_crowd_out_the_beads_own_id() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = Painted::of(
            bead_line(&row(&staffed), LAST, &ids(4), &Layout::default()),
            30,
            1,
        )
        .rows();

        assert!(drawn[0].contains(".20"), "{drawn:?}");
    }

    /// Narrow enough and the cell has no room at all. It goes whole rather
    /// than leaving a marker standing for a caption that is not there.
    #[test]
    fn a_caption_with_no_room_left_takes_the_whole_agent_cell_with_it() {
        let staffed = captioned("teach the elided run to fold back open on a keypress");

        let drawn = Painted::of(
            bead_line(&row(&staffed), LAST, &ids(4), &Layout::default()),
            14,
            1,
        )
        .rows();

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
                short: None,
                colour: None,
            },
            Badged {
                key: "blocked_on".into(),
                text: "⏸ waiting".into(),
                link: None,
                short: None,
                colour: None,
            },
        ];
        let drawn = Painted::of(
            bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
            100,
            1,
        )
        .rows();
        let first = drawn[0].find("⇢ #12").expect("the first badge");
        let second = drawn[0].find("⏸ waiting").expect("the second badge");

        assert!(first < second, "{drawn:?}");
    }

    /// The layout says where every cell goes, and the walk draws what it
    /// says: a badge named into the identity sits one column off the id and
    /// leaves the badges after the title; the agent named ahead of the
    /// fraction leads the state. The badge is still the link it was after
    /// the title, because a link belongs to its span and not to a block.
    #[test]
    fn a_row_is_drawn_in_whatever_order_its_layout_names() {
        let somewhere = "https://forge.invalid/orbital/atlas/pull/12";
        let mut badged = node("smt-4kd3p.2", "the noctalia widget", Status::InProgress);
        badged.badges = vec![
            Badged {
                key: "delivery_pr".into(),
                text: "⇢ #12".into(),
                link: Some(somewhere.into()),
                short: None,
                colour: None,
            },
            Badged {
                key: "jira".into(),
                text: "ATLAS-19".into(),
                link: None,
                short: None,
                colour: None,
            },
        ];
        let mut epic = row(&badged);
        epic.progress = Some(row::Progress {
            closed: 3,
            total: 8,
        });
        epic.agent = Some(row::agent_marker(&a_pane()));
        let layout = Layout {
            identity: vec![Cell::Glyph, Cell::Badge("delivery_pr".into()), Cell::Id],
            title: vec![Cell::Title, Cell::Badges],
            state: vec![Cell::Agent, Cell::Progress, Cell::Anomalies],
        };

        let drawn = Painted::of(bead_line(&epic, BRANCH, &ids(3), &layout), 80, 1).rows();
        let said = symbols(bead_line(&epic, BRANCH, &ids(3), &layout), 80);

        assert_eq!(
            drawn,
            vec![
                "  ├── ◐ ⇢ #12 .2   the noctalia widget  ATLAS-19         ◍ wCM:p9 · working  3/8"
            ]
        );
        assert!(
            said.contains(
                &hyperlink("⇢ #12", somewhere).expect("this vocabulary holds no control character")
            ),
            "the badge in the identity was drawn without its link: {said:?}"
        );
    }

    /// The identity pads every cell to its widest, not the id alone. A bead
    /// without the badge the layout put ahead of its id still gives the badge
    /// its column, so the ids under one another line up whether or not the
    /// bead above drew anything there.
    #[test]
    fn a_bead_without_a_badge_in_the_identity_leaves_its_column_blank() {
        let mut badged = node("smt-4kd3p.2", "the noctalia widget", Status::InProgress);
        badged.badges = vec![Badged {
            key: "delivery_pr".into(),
            text: "⇢ #12".into(),
            link: None,
            short: None,
            colour: None,
        }];
        let unbadged = node("smt-4kd3p.20", "wallpaper timer calls dms", Status::Open);
        let layout = Layout {
            identity: vec![Cell::Glyph, Cell::Badge("delivery_pr".into()), Cell::Id],
            ..Layout::default()
        };
        let widths = Widths::from([(Cell::Badge("delivery_pr".into()), 5), (Cell::Id, 3)]);

        let drawn = vec![
            Painted::of(bead_line(&row(&badged), BRANCH, &widths, &layout), 60, 1).rows()[0]
                .clone(),
            Painted::of(bead_line(&row(&unbadged), LAST, &widths, &layout), 60, 1).rows()[0]
                .clone(),
        ];

        assert_eq!(
            drawn,
            vec![
                "  ├── ◐ ⇢ #12 .2   the noctalia widget                      ",
                "  └── ○       .20  wallpaper timer calls dms                ",
            ]
        );
    }

    /// The table is in columns, as `fitted` measures a span, and not in
    /// characters: a badge whose render is one emoji two columns wide is
    /// padded to two, so the id after it does not land a column early.
    #[test]
    fn a_cell_is_padded_in_columns_rather_than_in_characters() {
        let mut badged = node("smt-4kd3p.2", "the noctalia widget", Status::InProgress);
        badged.badges = vec![Badged {
            key: "issue_type".into(),
            text: "🐛".into(),
            link: None,
            short: None,
            colour: None,
        }];
        let layout = Layout {
            identity: vec![Cell::Glyph, Cell::Badge("issue_type".into()), Cell::Id],
            ..Layout::default()
        };
        let measured = identity_widths(&row(&badged), &layout);
        let widths = Widths::from([(Cell::Badge("issue_type".into()), 2), (Cell::Id, 3)]);

        assert_eq!(measured.of(&Cell::Badge("issue_type".into())), 2);
        // The emoji's second column reads back as a blank of its own.
        assert_eq!(
            Painted::of(bead_line(&row(&badged), BRANCH, &widths, &layout), 40, 1).rows()[0],
            "  ├── ◐ 🐛  .2   the noctalia widget     "
        );
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
                short: None,
                colour: None,
            },
            Badged {
                key: "blocked_on".into(),
                text: "⏸ waiting".into(),
                link: None,
                short: None,
                colour: None,
            },
        ];

        let painted = Painted::of(
            bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
            100,
            1,
        );
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

    /// The whole of what `colour = "status"` buys: one badge, drawn on two
    /// beads, taking the colour each of them draws its own id in. A reader
    /// who has learned what an orange id means reads the badge beside it
    /// without being told anything further.
    #[test]
    fn a_badge_coloured_by_status_is_drawn_the_colour_of_the_beads_own_id() {
        let on = |status: Status| {
            let mut badged = node("smt-4kd3p.20", "a bead", status);
            badged.badges = vec![Badged {
                key: "jira".into(),
                text: "ATLAS-19".into(),
                link: None,
                short: None,
                colour: Some(Colour::Status),
            }];
            let painted = Painted::of(
                bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
                100,
                1,
            );
            (
                run_saying(&painted, ".20").style.fg,
                run_saying(&painted, "ATLAS-19").style.fg,
            )
        };

        let (blocked_id, blocked_badge) = on(Status::Blocked);
        let (going_id, going_badge) = on(Status::InProgress);

        assert_eq!(blocked_badge, blocked_id, "on a blocked bead");
        assert_eq!(going_badge, going_id, "on an in-progress one");
        assert_ne!(
            blocked_badge, going_badge,
            "and the same badge is a different colour on each"
        );
    }

    /// A badge whose config named no colour is drawn as it was before there
    /// was one to name: nothing of its own, so the tone of the row it sits on
    /// reaches it the way it reaches the title beside it.
    ///
    /// Asked of a blocked bead, which is the row that tells the two apart: on
    /// one whose status has no colour the badge is the row's tone either way,
    /// and the assertion would pass whichever it took.
    #[test]
    fn a_badge_that_names_no_colour_is_left_the_tone_of_the_row_it_sits_on() {
        let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
        badged.badges = vec![Badged {
            key: "jira".into(),
            text: "ATLAS-19".into(),
            link: None,
            short: None,
            colour: None,
        }];
        let row = row(&badged);

        let painted = Painted::of(bead_line(&row, BRANCH, &ids(4), &Layout::default()), 100, 1);
        let badge = run_saying(&painted, "ATLAS-19");

        assert_eq!(badge.style.fg, tone(&row).fg, "{badge:?}");
        assert_ne!(
            badge.style.fg,
            status_style(&Status::Blocked).fg,
            "a badge that named no colour took one anyway: {badge:?}"
        );
    }

    /// Neither knob has to know about the other. `palette::LINK` is an
    /// underline carrying no colour of its own, so a badge given both is
    /// underlined *and* coloured rather than one of them winning.
    #[test]
    fn a_badge_given_both_a_link_and_a_colour_is_underlined_in_that_colour() {
        let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
        badged.badges = vec![Badged {
            key: "jira".into(),
            text: "ATLAS-19".into(),
            link: Some("https://forge.invalid/browse/ATLAS-19".into()),
            short: None,
            colour: Some(Colour::Status),
        }];

        let painted = Painted::of(
            bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
            100,
            1,
        );
        let badge = run_saying(&painted, "ATLAS-19");

        assert!(
            badge.style.add_modifier.contains(Modifier::UNDERLINED),
            "the colour took the underline with it: {badge:?}"
        );
        assert_eq!(
            badge.style.fg,
            status_style(&Status::Blocked).fg,
            "the underline took the colour with it: {badge:?}"
        );
    }

    /// The whole point of the rule is that the reader sees something, so the
    /// words have to survive the trip to the buffer rather than stopping at
    /// the row.
    #[test]
    fn a_badge_that_fell_short_of_its_config_says_so_on_the_drawn_row() {
        let mut short = row(&node("smt-4kd3p.20", "a bead", Status::Blocked));
        short.notes = vec![
            phrase::undrawn(&Undrawn::Link {
                key: "delivery_pr".into(),
            }),
            phrase::unopenable_link("jira"),
        ];

        let drawn = Painted::of(
            bead_line(&short, BRANCH, &ids(3), &Layout::default()),
            160,
            1,
        )
        .rows();

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
            short: None,
            colour: None,
        }];

        let painted = Painted::of(
            bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
            100,
            1,
        );
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
            short: None,
            colour: None,
        };
        let linked = Badged {
            link: Some("https://forge.invalid/orbital/atlas/pull/12".into()),
            short: None,
            colour: None,
            ..unlinked.clone()
        };

        let said = |badge: Badged| {
            let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
            badged.badges = vec![badge];
            Painted::of(
                bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
                EXACTLY_THE_ROW,
                1,
            )
            .rows()
        };

        let drawn = said(linked);

        assert!(
            drawn[0].ends_with("a bead  ⇢ #12"),
            "the badge did not survive the row's own width: {drawn:?}"
        );
        assert_eq!(drawn, said(unlinked));
    }

    /// A badge the row cut still points where it always did, so it is still
    /// drawn as a link. The words are clipped; the destination is not.
    ///
    /// Read as a pair on one badge rather than as a reading at the narrow
    /// width alone. A badge that never carried a link is not underlined at
    /// any width, so the narrow reading on its own passes whether the
    /// underline survived the cut or was never there.
    ///
    /// The badge that never carried one is read at both widths, as a whole
    /// style rather than for its underline. The cut moves nothing about a
    /// badge's style, and a cut that reached the badge's colour would move a
    /// badge no link was ever drawn on.
    #[test]
    fn a_badge_the_row_cut_is_still_drawn_as_the_link_it_still_is() {
        let badge_at = |width: u16, to: Option<&str>| {
            let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
            badged.badges = vec![Badged {
                key: "delivery_pr".into(),
                text: "⇢ #12".into(),
                link: to.map(str::to_string),
                short: None,
                colour: Some(Colour::Status),
            }];
            let painted = Painted::of(
                bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
                width,
                1,
            );
            run_saying(&painted, "⇢ #").style
        };
        let somewhere = Some("https://forge.invalid/orbital/atlas/pull/12");

        assert!(
            badge_at(EXACTLY_THE_ROW, somewhere)
                .add_modifier
                .contains(Modifier::UNDERLINED),
            "the badge the row kept whole is not underlined"
        );
        assert!(
            badge_at(EXACTLY_THE_ROW - 1, somewhere)
                .add_modifier
                .contains(Modifier::UNDERLINED),
            "the cut took the underline off a badge that still opens"
        );
        assert_eq!(
            badge_at(EXACTLY_THE_ROW - 1, None),
            badge_at(EXACTLY_THE_ROW, None),
            "the cut moved a badge that never carried a link"
        );
    }

    /// The underline is only a promise; what the reader acts on is the
    /// hyperlink. A cut badge that kept the one without the other would
    /// invite the click and drop it.
    ///
    /// Read at the head the row kept rather than at the badge's whole words,
    /// because the sequence has to close inside the columns the row still
    /// has.
    #[test]
    fn a_badge_the_row_cut_opens_where_a_badge_it_kept_whole_opens() {
        let somewhere = "https://forge.invalid/orbital/atlas/pull/12";
        let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
        badged.badges = vec![Badged {
            key: "delivery_pr".into(),
            text: "⇢ #12".into(),
            link: Some(somewhere.into()),
            short: None,
            colour: None,
        }];

        let said = symbols(
            bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
            EXACTLY_THE_ROW - 1,
        );

        assert!(
            said.contains(
                &hyperlink("⇢ #", somewhere).expect("this vocabulary holds no control character")
            ),
            "the badge the row cut was not made a link: {said:?}"
        );
    }

    /// A badge whose words the emitter refuses is not opened at any width,
    /// including the widths that cut those words back to a head it would
    /// accept. The row would otherwise open a link it drew no underline on,
    /// which is the opposite mistake to the one the cut used to make and just
    /// as much of a lie.
    ///
    /// Swept over every width rather than read at the one that cuts, because
    /// which width that is falls out of a control character's own zero
    /// columns. The clean badge is swept alongside it, so a sweep that opened
    /// nothing anywhere cannot pass as a sweep that refused.
    ///
    /// Read without the note the refused badge earns the row, which would
    /// otherwise take the columns the sweep is spending on the badge. What
    /// the note says is `row::cells`' to say and is read there.
    #[test]
    fn a_badge_the_emitter_refuses_is_not_opened_at_the_widths_that_cut_it() {
        let somewhere = "https://forge.invalid/orbital/atlas/pull/12";
        let held = "\u{1b}]0;owned\u{7}";
        let opened_at = |text: String, width: u16| {
            let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
            badged.badges = vec![Badged {
                key: "delivery_pr".into(),
                text,
                link: Some(somewhere.into()),
                short: None,
                colour: None,
            }];
            let mut unremarked = row(&badged);
            unremarked.notes = Vec::new();
            symbols(
                bead_line(&unremarked, BRANCH, &ids(4), &Layout::default()),
                width,
            )
            .contains(somewhere)
        };
        let every_width = || 1..=60;

        assert!(
            every_width().any(|width| opened_at("⇢ #12".into(), width)),
            "the sweep opened no link at any width, so it refuses nothing"
        );
        assert!(
            !every_width().any(|width| opened_at(format!("⇢ #12{held}"), width)),
            "a badge the emitter refuses whole was opened by a cut"
        );
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
            short: None,
            colour: None,
        };
        let said = |badge: Badged| {
            let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
            badged.badges = vec![badge];
            symbols(
                bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
                100,
            )
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

    /// A badge saying `⇢ atlas #12`, with `⇢ #12` to fall back to. The long
    /// form is a tracker's length to choose and the short one is the config's,
    /// so neither is `bdi`'s and the row can only pick between them.
    fn shortenable(to: Option<&str>) -> Badged {
        Badged {
            key: "delivery_pr".into(),
            text: "⇢ atlas #12".into(),
            short: Some("⇢ #12".into()),
            link: to.map(str::to_string),
            colour: None,
        }
    }

    fn a_row_badged(badge: Badged, width: u16) -> String {
        let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
        badged.badges = vec![badge];
        Painted::of(
            bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
            width,
            1,
        )
        .rows()[0]
            .clone()
    }

    /// One column narrower than the long form needs and the badge says itself
    /// shortly, whole, rather than being cut to a head that names no pull
    /// request. This is the width a reader actually runs: a title and one
    /// reference, in a pane that is a little too narrow for both.
    #[test]
    fn a_badge_too_wide_for_the_row_is_said_in_the_short_form_its_config_named() {
        assert_eq!(
            a_row_badged(shortenable(None), 33),
            "  ├── ● .20   a bead  ⇢ atlas #12"
        );
        assert_eq!(
            a_row_badged(shortenable(None), 32),
            "  ├── ● .20   a bead  ⇢ #12     "
        );
    }

    /// The same row at the same width with nothing offered in its place, which
    /// is every badge configured before there was a short form to name.
    #[test]
    fn a_badge_offering_no_short_form_is_cut_as_it_always_was() {
        let one_length = Badged {
            short: None,
            ..shortenable(None)
        };

        assert_eq!(
            a_row_badged(one_length, 32),
            "  ├── ● .20   a bead  ⇢ atlas #…"
        );
    }

    /// A short form the row can afford is a span it kept whole, so the badge
    /// is still followable at exactly the widths it shortened in order to
    /// survive. Lose the link here and shortening would trade the reference
    /// for the words that name it.
    #[test]
    fn a_badge_said_in_its_short_form_still_points_where_the_long_one_did() {
        let somewhere = "https://forge.invalid/orbital/atlas/pull/12";
        let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
        badged.badges = vec![shortenable(Some(somewhere))];

        let said = symbols(
            bead_line(&row(&badged), BRANCH, &ids(4), &Layout::default()),
            32,
        );

        assert!(
            said.contains(
                &hyperlink("⇢ #12", somewhere).expect("this vocabulary holds no control character")
            ),
            "the short form was drawn without the link the long one had: {said:?}"
        );
    }

    /// A short form buys the badge widths, not every width. Below its own
    /// length there is nothing left to swap in, so the row cuts it exactly as
    /// it cuts a badge that never offered one — and the two agree cell for
    /// cell, which is the reading that says the short form stopped mattering
    /// rather than that it changed what a narrow row does.
    #[test]
    fn a_badge_too_wide_for_its_short_form_too_is_cut_as_one_offering_none_is() {
        let one_length = Badged {
            short: None,
            ..shortenable(None)
        };

        assert_eq!(
            a_row_badged(shortenable(None), 24),
            a_row_badged(one_length, 24)
        );
        assert_eq!(
            a_row_badged(shortenable(None), 24),
            "  ├── ● .20   a bead  ⇢…"
        );
    }

    /// The row picks a badge's form by its width, and picks the style once for
    /// both: `badge_style` is asked before anything has been fitted, so it can
    /// only read the long form. Two forms that disagree about whether the link
    /// can be written would make the row's answer depend on how wide it is —
    /// underlined at one width and openable at another, and each without the
    /// other. A badge is a link or it is not, so the row keeps the one length
    /// it can say that about.
    ///
    /// Read both ways round, because the two disagreements are opposite
    /// mistakes and one rule has to cover both. A short form the emitter would
    /// refuse would be underlined and dead; a clean short form on a badge whose
    /// long form is refused would open a page nothing marked as a link.
    #[test]
    fn a_badge_whose_two_forms_disagree_about_its_link_keeps_one_length() {
        let somewhere = "https://forge.invalid/orbital/atlas/pull/12";
        let held = "\u{1b}]0;owned\u{7}";
        // Read without the notes the row would carry beside them. What each
        // badge leaves on the row is `row::cells`' to say and is read there;
        // here the question is only which form the row drew.
        let unremarked = |badge: Badged, width: u16| {
            let mut badged = node("smt-4kd3p.20", "a bead", Status::Blocked);
            badged.badges = vec![badge];
            let mut row = row(&badged);
            row.notes = Vec::new();
            Painted::of(
                bead_line(&row, BRANCH, &ids(4), &Layout::default()),
                width,
                1,
            )
            .rows()[0]
                .clone()
        };
        let one_length = |badge: Badged| {
            unremarked(
                Badged {
                    short: None,
                    ..badge
                },
                32,
            )
        };

        let refused_short = Badged {
            short: Some(format!("⇢ #12{held}")),
            ..shortenable(Some(somewhere))
        };
        assert_eq!(
            unremarked(refused_short.clone(), 32),
            one_length(refused_short),
            "a short form the emitter would refuse was drawn anyway"
        );

        let refused_long = Badged {
            text: format!("⇢ atlas #12{held}"),
            ..shortenable(Some(somewhere))
        };
        assert_eq!(
            unremarked(refused_long.clone(), 32),
            one_length(refused_long),
            "a badge whose link was refused was made one by shortening"
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
        let drawn = Painted::of(
            bead_line(&row(&long), BRANCH, &ids(4), &Layout::default()),
            40,
            3,
        )
        .rows();

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
        let painted = Painted::of(
            fitted(
                &under(BRANCH, elided(15)),
                &ids(0),
                &Layout::default(),
                &at_rest(),
            ),
            72,
            1,
        )
        .row(0);

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
        let painted = Painted::of(
            fitted(
                &under(BRANCH, elided(3)),
                &ids(0),
                &Layout::default(),
                &at_rest(),
            ),
            72,
            1,
        )
        .row(0);

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
                &ids(3),
                &Layout::default(),
            );
            let painted = Painted::of(drawn, 60, 1).row(0);

            let id = painted
                .iter()
                .find(|run| run.said.contains(".2"))
                .expect("the id is drawn");
            let glyph = painted
                .iter()
                .find(|run| run.said.starts_with(row::status_glyph(&status)))
                .expect("the glyph is drawn");
            assert_eq!(id.style.fg, status_style(&status).fg, "{painted:?}");
            assert_eq!(glyph.style.fg, id.style.fg, "{painted:?}");
        }
    }

    /// Open is the one status `bd` gives no colour of its own, so the id has
    /// none either and the row's own tone reaches it as it does the rest.
    #[test]
    fn an_open_beads_id_is_left_in_the_colour_the_rest_of_its_row_is_in() {
        let node = node("smt-4kd3p.2", "a bead", Status::Open);

        let painted = Painted::of(
            bead_line(&row(&node), BRANCH, &ids(3), &Layout::default()),
            60,
            1,
        )
        .row(0);

        let id = painted
            .iter()
            .find(|run| run.said.contains(".2"))
            .expect("the id is drawn");
        assert_eq!(id.style.fg, Some(Color::Reset), "{painted:?}");
    }
}
