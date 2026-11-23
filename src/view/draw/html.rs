//! The forest and the bead drawn as a web page rather than into a frame.
//!
//! Every line is the line the terminal draws, in its words and its colours,
//! written as HTML. What the terminal does with a key the page does with a
//! link, and every link's address is `view::address`'s to compose: a line
//! that folds links to the page with that fold toggled, and every reference
//! to a bead, a row of the forest or a bead the bead view names, is one
//! component that opens the bead.

use std::collections::HashMap;

use chrono::{DateTime, FixedOffset, Utc};
use ratatui::text::Span;

use crate::app::Awaited;
use crate::model::edges::Related;
use crate::model::join::BeadKey;
use crate::model::snapshot::{Node, Snapshot};
use crate::model::types::{Edge, Status};
use crate::view::address::{line_name, Asked, FIND, SHOWN};
use crate::view::fitted::{indent, Block, Mark, Marked};
use crate::view::forest::{listed, only_ready, view_of, Forest, Landed};
use crate::view::lines::{self, Content, Level, Line, Note, ProjectLine, Waiting};
use crate::view::markdown::Flow;
use crate::view::row::{Layout, WARNING};
use crate::view::{palette, phrase, show, Said};

use super::bead::referenced_line;
use super::foot::notices;
use super::project::wants_attention;
use super::{fitted, Reads, Reference};

/// The columns the bead view wraps its prose at, which is what `bd show`
/// wraps at.
const PROSE: usize = 80;

/// The page a request asked for, and whether it holds what was asked for: a
/// bead's address names a bead the forest draws, or it does not.
pub struct Answer {
    pub found: bool,
    pub html: String,
}

/// When a page is drawn, the offset from UTC it says the time of day at, the
/// reads out as it is drawn, and the change to what the server holds that
/// it is drawn from, which its script asks for the changes since.
#[derive(Debug, Clone)]
pub struct Drawn {
    pub at: DateTime<Utc>,
    pub offset: FixedOffset,
    pub collecting: Vec<Awaited>,
    pub change: u64,
}

/// The page `asked` names, drawn from `snapshot` under `layout`.
///
/// A page may be left open with no script to keep it current, so each
/// project's line says the time of day it was read at rather than how long
/// ago, which the terminal says because it redraws itself as the age moves.
pub fn page(snapshot: Snapshot, layout: &Layout, asked: &Asked, drawn: Drawn) -> Answer {
    let shown = asked.forest(snapshot, layout);
    let forest = &shown.forest;
    let node = show::selected(forest).filter(|_| shown.bead == Some(true));
    let asked = &shown.asked;
    let matched = matches!(shown.landed, Some(Landed::On { .. }));
    let drawing = Drawing {
        forest,
        layout,
        asked,
        selected: (node.is_some() || matched).then(|| forest.selected_line()),
    };

    let title = node.map_or_else(
        || phrase::PRODUCT.to_string(),
        |node| phrase::bead_page_title(&node.title),
    );
    let mut html = head(&title, Some(drawn.change));
    html.push_str("<body>");
    drawing.nav(&mut html);
    html.push_str(if shown.bead.is_some() {
        "<main class=\"open\"><div class=\"forest\">"
    } else {
        "<main><div class=\"forest\">"
    });
    drawing.forest_lines(&mut html, drawn);
    html.push_str("</div>");
    if let Some(key) = &asked.bead {
        html.push_str(&format!(
            "<aside><a class=\"close\" href=\"{}\">{}</a>\n",
            escaped(&asked.closing(node.and(forest.place()), forest.list().is_some())),
            escaped(phrase::back())
        ));
        match node {
            Some(node) => {
                drawing.rooting(&mut html);
                drawing.bead_view(&mut html, node);
            }
            None => html.push_str(&format!(
                "<p class=\"missing\">{}</p>",
                escaped(&format!("{WARNING} {}", phrase::not_drawn(key)))
            )),
        }
        html.push_str("</aside>");
    }
    html.push_str("</main>");
    drawing.foot(&mut html, shown.landed, &shown.searched);
    html.push_str("</body></html>\n");
    Answer {
        found: asked.bead.is_none() || node.is_some(),
        html,
    }
}

/// A page with nothing on it but one sentence and a link to the forest, for
/// an address that has no forest to draw.
pub fn saying(said: &str) -> String {
    said_alone(said, None)
}

/// The same, for an address that will have a forest to draw once the server
/// holds a later change than `change`, which the page is kept current to.
pub fn saying_kept_current(said: &str, change: u64) -> String {
    said_alone(said, Some(change))
}

fn said_alone(said: &str, change: Option<u64>) -> String {
    format!(
        "{}<body><footer><p>{}</p>\
         <p><a class=\"close\" href=\"/\">{}</a></p></footer></body></html>\n",
        head(phrase::PRODUCT, change),
        escaped(said),
        escaped(phrase::back())
    )
}

/// A page's head, titled `title`. Where the page is drawn from `change`, it
/// carries the script that keeps it current.
fn head(title: &str, change: Option<u64>) -> String {
    let (named, script) = match change {
        Some(change) => (
            format!(" data-change=\"{change}\""),
            format!("<script>{KEPT_CURRENT}</script>"),
        ),
        None => (String::new(), String::new()),
    };
    format!(
        "<!doctype html><html lang=\"en\"{named}><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{}{STYLE}</style>{script}</head>",
        escaped(title),
        palette::page_colours()
    )
}

/// The page as it is being drawn: the forest, the row layout and what was
/// asked, and the line the bead asked for is on.
struct Drawing<'a> {
    forest: &'a Forest,
    layout: &'a Layout,
    asked: &'a Asked,
    selected: Option<usize>,
}

impl Drawing<'_> {
    /// What the terminal's keys put in force over the whole page, as links
    /// and a form: the list menu where the config names views, each with how
    /// many beads it lists, the ready switch and the search, as `v`, `r` and
    /// `/`. Then, where the forest is
    /// drawn with one bead as its only root, that bead and the link that
    /// puts every tree back, as `F` does.
    fn nav(&self, html: &mut String) {
        let list = self.forest.list();
        html.push_str(&format!(
            "<nav><span class=\"brand\">{}</span>",
            phrase::PRODUCT
        ));
        let views = &self.forest.snapshot().views;
        if !views.is_empty() {
            let ready = only_ready(list);
            let drawn = view_of(list);
            html.push_str(&format!(
                "<details class=\"lists\"><summary><span>{}</span></summary><div>",
                escaped(drawn.unwrap_or(phrase::every_tree()))
            ));
            html.push_str(&format!(
                "<a href=\"{}\"{}>{}</a><p class=\"hd\">{}</p>",
                escaped(&self.asked.listing(listed(None, ready))),
                on(drawn.is_none()),
                escaped(phrase::every_tree()),
                escaped(phrase::views())
            ));
            for (view, count) in views.iter().zip(self.forest.view_counts(ready)) {
                html.push_str(&format!(
                    "<a href=\"{}\"{}>{}<span class=\"count\">{count}</span></a>",
                    escaped(&self.asked.listing(listed(Some(view.clone()), ready))),
                    on(Some(view.as_str()) == drawn),
                    escaped(view)
                ));
            }
            html.push_str("</div></details>");
        }
        let ready = only_ready(list);
        html.push_str(&format!(
            "<a class=\"ready{}\" href=\"{}\">{}</a>",
            if ready { " on" } else { "" },
            escaped(&self.asked.toggling_ready()),
            escaped(phrase::ready_only())
        ));
        html.push_str(&format!("<form action=\"/#{SHOWN}\" method=\"get\">"));
        // A search that found nothing put the selection back where it began,
        // which the next search counts from, though no line is marked.
        let counted_from = (self.selected.is_some() || self.asked.find.is_some())
            .then(|| self.forest.place())
            .flatten();
        for (key, value) in self.asked.carried(counted_from) {
            html.push_str(&format!(
                "<input type=\"hidden\" name=\"{key}\" value=\"{}\">",
                escaped(&value)
            ));
        }
        let typed = self.asked.find.as_ref().map_or("", |find| &find.text);
        html.push_str(&format!(
            "<label><input type=\"search\" name=\"{FIND}\" value=\"{}\" aria-label=\"{}\" \
             placeholder=\"{}\"><span>{}</span></label><button>{}</button></form>",
            escaped(typed),
            escaped(phrase::find()),
            escaped(phrase::find_prompt()),
            escaped(phrase::find()),
            escaped(phrase::find())
        ));
        html.push_str("</nav>\n");
        let Some(root) = self.asked.root.as_ref() else {
            return;
        };
        // An address can name a root the forest no longer draws, and the
        // page then draws every tree, so it says so rather than drop it.
        let said = if self.forest.is_focused() {
            phrase::showing_only().to_string()
        } else {
            format!("{WARNING} {}", phrase::not_drawn(root.key()))
        };
        html.push_str(&format!(
            "<p class=\"rooted\"><span class=\"said\">{}</span> ",
            escaped(&said)
        ));
        if let Some(node) = self
            .forest
            .snapshot()
            .node(root.key())
            .filter(|_| self.forest.is_focused())
        {
            self.reference(
                html,
                show::named(node, vec![Span::raw(node.title.clone())]),
                Some(self.opening(root.key())),
            );
        }
        html.push_str(&format!(
            " <a class=\"all\" href=\"{}\">{}</a></p>\n",
            escaped(&self.asked.rooting(None)),
            escaped(phrase::show_all())
        ));
    }

    /// The link on the bead view that draws the bead as the only root, as
    /// `F` does, where the forest is drawn whole. Drawn with one root, the
    /// key puts every tree back, which the page's head offers.
    fn rooting(&self, html: &mut String) {
        let Some(place) = self.forest.place().filter(|_| !self.forest.is_focused()) else {
            return;
        };
        html.push_str(&format!(
            "<a class=\"root\" href=\"{}\">{}</a>\n",
            escaped(&self.asked.rooting(Some(place))),
            escaped(phrase::only_this_tree())
        ));
    }

    fn forest_lines(&self, html: &mut String, drawn: Drawn) {
        let lines = self.forest.lines();
        let reads = Reads::new(
            &self.forest.snapshot().read_at,
            &drawn.collecting,
            &[],
            drawn.at,
        );
        for (at, line) in lines.viewport(0, lines.len()) {
            match &line.content {
                // Said at the foot, with the link that shows those beads.
                Content::Note(Note::ToCheck(_)) => {}
                Content::Project(project) => {
                    self.open_line(html, at, line, " heading", String::new());
                    self.heading(html, line, project, &reads, drawn.offset);
                }
                _ => self.forest_line(html, at, line, &reads),
            }
        }
    }

    /// A line's opening tag, classed `kind` and styled `css`, and the anchor
    /// a bead's page lands on where the line is the one it asked for.
    fn open_line(&self, html: &mut String, at: usize, line: &Line, kind: &str, css: String) {
        let selected = self.selected == Some(at);
        let anchor = line_name(line, self.listed())
            .map_or_else(String::new, |name| format!(" id=\"{name}\""));
        html.push_str(&format!(
            "<div class=\"line{kind}{}\"{anchor}{}>",
            if selected { " shown" } else { "" },
            styled(css)
        ));
        if selected {
            html.push_str(&format!("<a id=\"{SHOWN}\"></a>"));
        }
    }

    /// A project's line as the heading over its trees: its name, when it was
    /// read, how far through it is, the agents on it and the beads in it to
    /// check. The terminal says the same on the project's line.
    fn heading(
        &self,
        html: &mut String,
        line: &Line,
        project: &ProjectLine,
        reads: &Reads,
        offset: FixedOffset,
    ) {
        html.push_str("<span class=\"left\">");
        self.prefix(html, line, &Span::raw(line.prefix.clone()));
        html.push_str(&format!("<h2>{}</h2></span>", escaped(&project.project)));
        if let Some(how_fresh) = reads.of(project) {
            let tone = if wants_attention(how_fresh.mark) {
                " warn"
            } else if how_fresh.mark == crate::view::Mark::Read {
                " ok"
            } else {
                ""
            };
            html.push_str(&format!(
                "<span class=\"read\"><span class=\"mark{tone}\">{}</span>",
                escaped(phrase::mark(how_fresh, reads.now))
            ));
            if let (Some(when), Some(at)) = (
                phrase::read_at(how_fresh, offset, reads.now),
                how_fresh.read_at,
            ) {
                html.push_str(&format!(
                    " <time datetime=\"{}\" title=\"{}\">{}</time>",
                    at.to_rfc3339(),
                    escaped(&phrase::read_in_full(at, offset)),
                    escaped(&when)
                ));
            }
            html.push_str("</span>");
        }
        let counts = &project.counts;
        html.push_str("<span class=\"state\">");
        // Nothing counted means no root here read at all, which a bar would
        // say the opposite of.
        if let Some(percent) = (counts.finished * 100).checked_div(counts.total) {
            html.push_str(&format!(
                "<span class=\"prog\"><span class=\"bar\" aria-hidden=\"true\">\
                 <span style=\"width:{percent}%\"></span></span>{}</span>",
                escaped(&phrase::done_of(counts.finished, counts.total))
            ));
        }
        if counts.live_agents > 0 {
            html.push_str(&format!(
                "<span class=\"agents\">{}</span>",
                escaped(&phrase::agents(counts.live_agents))
            ));
        }
        if counts.anomalies > 0 {
            html.push_str(&format!(
                "<span class=\"flag\">{}</span>",
                escaped(&format!(
                    "{WARNING} {}",
                    phrase::anomalies_beneath(counts.anomalies)
                ))
            ));
        }
        html.push_str("</span></div>\n");
    }

    /// One line of the forest. A bead's row is the terminal's row with its
    /// glyph, id and title drawn as a reference to the bead.
    fn forest_line(&self, html: &mut String, at: usize, line: &Line, reads: &Reads) {
        let widths = self.forest.lines().widths();
        let (reference, drawn) = match &line.content {
            Content::Bead(row) | Content::Waiting(Waiting { row, .. }) => {
                let (reference, drawn) = referenced_line(row, &line.prefix, widths, self.layout);
                (Some(reference), drawn)
            }
            _ => (None, fitted(line, widths, self.layout, reads)),
        };
        let whole = drawn.whole();
        self.open_line(html, at, line, "", palette::css(whole.under));
        let (mut identity, mut from) = (whole.identity, 0);
        let prefix = identity
            .split_first()
            .filter(|(prefix, _)| prefix.content == line.prefix);
        if let Some((_, rest)) = prefix {
            (identity, from) = (rest, 1);
        }
        // A bead's line keeps apart the parts a phone lays out on two lines:
        // what it hangs under, the reference, the row's other cells and its
        // state. Any other line is drawn as what it hangs under, then one run
        // beside it, before its state.
        let bead = reference.is_some();
        let more = bead && !(identity.is_empty() && whole.title.is_empty());
        if !bead {
            html.push_str("<span class=\"left\">");
        }
        if let Some((prefix, _)) = prefix {
            self.prefix(html, line, prefix);
        }
        if !bead {
            html.push_str("<span>");
        }
        if let Some(reference) = reference {
            // A bead's own row is a line this page draws, so the page it
            // opens draws it too. A row waiting under a list's row is not.
            let href = line.place.as_ref().map(|place| match line.content {
                Content::Bead(_) => self.asked.opening_at(place),
                _ => self.opening(place.key()),
            });
            self.reference(html, reference, href);
        }
        if more {
            html.push_str("<span class=\"more\">");
        }
        if bead && !identity.is_empty() {
            html.push_str(&indent());
        }
        spans(html, identity, Block::Identity, whole.marks, from);
        if !whole.title.is_empty() {
            html.push_str(&indent());
            spans(html, whole.title, Block::Title, whole.marks, 0);
        }
        if !bead {
            html.push_str("</span></span>");
        }
        if more {
            html.push_str("</span>");
        }
        html.push_str("<span class=\"state\">");
        spans(html, whole.state, Block::State, whole.marks, 0);
        html.push_str("</span></div>\n");
    }

    /// Whether the lines are a list's rows, drawn in place of the trees.
    fn listed(&self) -> bool {
        self.forest.list().is_some()
    }

    /// The box-drawing a line hangs under, which is the link that toggles the
    /// line's fold where it has one.
    fn prefix(&self, html: &mut String, line: &Line, prefix: &Span<'static>) {
        let (drawn, rails) = match railed(&prefix.content, self.listed()) {
            Some(drawn) => (drawn, " rails"),
            None => (escaped(&prefix.content), ""),
        };
        let Some(href) = self.asked.toggling(line, self.listed()) else {
            let class = if rails.is_empty() {
                ""
            } else {
                " class=\"rails\""
            };
            html.push_str(&format!(
                "<span{class}{}>{drawn}</span>",
                styled(palette::css(prefix.style)),
            ));
            return;
        };
        let (said, shut) = if line.folded == Some(true) {
            (phrase::shut_the_fold(), "")
        } else {
            (phrase::open_the_fold(), " shut")
        };
        html.push_str(&format!(
            "<a class=\"fold{shut}{rails}\" href=\"{}\" aria-label=\"{}\"{}>{drawn}</a>",
            escaped(&href),
            escaped(said),
            styled(palette::css(prefix.style)),
        ));
    }

    /// The one component every reference to a bead is drawn by: its status
    /// mark, its title, then its id set quietly, and a link to `href` where
    /// the page can open the bead. The mark is labelled with `bd`'s word for
    /// the status. The id takes no colour from the place drawing it, because
    /// a status colour or a finished row's tone would make it fainter still.
    fn reference(&self, html: &mut String, reference: Reference, href: Option<String>) {
        let linked = href.is_some();
        match href {
            Some(href) => html.push_str(&format!("<a class=\"bead\" href=\"{}\">", escaped(&href))),
            None => html.push_str("<span class=\"bead\">"),
        }
        let between = |html: &mut String, gap: &str| {
            spans(
                html,
                &[Span::styled(gap.to_string(), reference.between)],
                Block::Identity,
                &[],
                0,
            )
        };
        let glyph = reference.glyph();
        html.push_str(&format!(
            "<span class=\"g{}\" role=\"img\" aria-label=\"{}\"{}>{}</span>",
            drawn_mark(&reference.status).map_or_else(String::new, |mark| format!(" {mark}")),
            escaped(&phrase::status_word(&reference.status)),
            styled(palette::css(glyph.style)),
            escaped(&glyph.content)
        ));
        between(html, " ");
        html.push_str("<span class=\"title\">");
        spans(html, &reference.title, Block::Identity, &[], 0);
        html.push_str("</span>");
        between(html, &indent());
        let id: String = reference
            .id
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        html.push_str(&format!(
            "<span class=\"id\">{}</span>",
            escaped(id.trim_end())
        ));
        html.push_str(if linked { "</a>" } else { "</span>" });
    }

    /// The bead the selection is on, as the terminal's bead view draws it:
    /// its head drawn as a reference to itself, and every bead it names drawn
    /// as a reference to that bead.
    fn bead_view(&self, html: &mut String, node: &Node) {
        let forest = self.forest;
        let mentioned = show::mentioned(forest, node);
        let followable = |related: &Related| show::followable(forest, related);
        let page = show::said(
            node,
            &mentioned,
            show::fraction(forest),
            PROSE,
            usize::MAX,
            &followable,
        );
        let named: HashMap<usize, &Related> = page
            .related
            .iter()
            .copied()
            .zip(show::related(node, &mentioned))
            .collect();

        html.push_str("<div class=\"page\"><div class=\"head\">");
        self.reference(
            html,
            show::named(node, vec![Span::raw(node.title.clone())]),
            forest.place().map(|place| self.asked.opening_at(place)),
        );
        html.push_str("</div>");
        let mut in_prose = false;
        for (at, row) in page.rows.iter().enumerate().skip(page.head) {
            let marks = page.marks.get(&at).map_or(&[][..], Vec::as_slice);
            // A row of prose starts with two spans: the indent the page gives
            // it, and the lead or the hang the text gives it.
            let (lead, rest) = row.split_at(row.len().min(2));
            let flow = page.prose.get(&at);
            // A row of prose broken from the one above only to fit the
            // terminal's width keeps that break in an element of its own, so a
            // page narrower than the terminal can let the prose flow.
            if let Some(Flow::Joins(joint)) = flow {
                html.push_str(joint);
                html.push_str("<span class=\"w\">\n");
                spans(html, lead, Block::Identity, marks, 0);
                html.push_str("</span>");
                spans(html, rest, Block::Identity, marks, lead.len());
                continue;
            }
            if std::mem::take(&mut in_prose) {
                html.push_str("</span></span>");
            }
            html.push('\n');
            // A line of prose is an element of its own, with what it starts
            // with apart from its text, so a phone can set the text in a face
            // of its own and hang every row of it under the text's start.
            if let Some(Flow::Opens) = flow {
                html.push_str("<span class=\"p\"><span class=\"lead\">");
                spans(html, lead, Block::Identity, marks, 0);
                html.push_str("</span><span class=\"t\">");
                spans(html, rest, Block::Identity, marks, lead.len());
                in_prose = true;
            } else if let Some(related) = named.get(&at) {
                let (arrow, rest) = row.split_at(row.len().min(2));
                html.push_str("<span class=\"arrow\">");
                spans(html, arrow, Block::Identity, &[], 0);
                html.push_str("</span>");
                if !self.related(html, related) {
                    spans(html, rest, Block::Identity, &[], 0);
                }
            } else if page.blocked_by == Some(at) {
                html.push_str(&indent());
                html.push_str(&escaped(phrase::blocked_by_ahead()));
                for (nth, blocker) in node.blocked_by.iter().enumerate() {
                    if nth > 0 {
                        html.push_str(&escaped(phrase::BETWEEN_BLOCKERS));
                    }
                    let named = blocking(forest.snapshot(), node, blocker);
                    if named.is_empty() {
                        html.push_str(&escaped(blocker));
                    }
                    for (nth, related) in named.iter().enumerate() {
                        if nth > 0 {
                            html.push_str(&escaped(phrase::BETWEEN_BLOCKERS));
                        }
                        if !self.related(html, related) {
                            html.push_str(&escaped(blocker));
                        }
                    }
                }
            } else {
                spans(html, row, Block::Identity, marks, 0);
            }
        }
        if in_prose {
            html.push_str("</span></span>");
        }
        html.push_str("\n</div>");
    }

    /// A bead the bead view names, as a reference to it that opens it where
    /// the forest draws it. Reports whether there was one to draw: a bead the
    /// answer does not hold has no glyph and no title.
    fn related(&self, html: &mut String, related: &Related) -> bool {
        let followable = show::followable(self.forest, related);
        let Some(reference) = show::related_reference(related, followable) else {
            return false;
        };
        let key = show::key_of(self.forest, related).filter(|_| followable);
        self.reference(html, reference, key.map(|key| self.opening(&key)));
        true
    }

    /// The address of the page showing `key`, which a reference with no
    /// row of its own on this page links to. In a list it lands on the copy
    /// the list has a row for. A bead outside the tree the forest is drawn
    /// with as its only root, or without a row in the list drawn, opens with
    /// every tree drawn, because this page has no line to show it on.
    fn opening(&self, key: &BeadKey) -> String {
        match self.forest.reaches(key) {
            Some(place) if self.forest.list().is_some() => self.asked.opening_at(&place),
            Some(_) => self.asked.opening(key),
            None => Asked {
                root: None,
                list: None,
                ..self.asked.clone()
            }
            .opening(key),
        }
    }

    /// Every notice the terminal says at its foot, one to a bar, after what
    /// it says of where a search came to, with the links that step to the
    /// match before and the match after, as `N` and `n` do. Under a list,
    /// the beads to check that it leaves out are a bar too, with a link to
    /// the trees, whose rows say what is wrong with them.
    fn foot(&self, html: &mut String, landed: Option<Landed>, searched: &Asked) {
        let notices = notices(self.forest.snapshot(), &[]);
        let to_check = self
            .forest
            .lines()
            .iter()
            .find_map(|line| match line.content {
                Content::Note(Note::ToCheck(count)) => Some(count),
                _ => None,
            });
        if notices.is_empty() && landed.is_none() && to_check.is_none() {
            return;
        }
        html.push_str("<footer>");
        if let Some(landed) = landed {
            let steps = match (&landed, &searched.find, self.forest.place()) {
                (Landed::On { of, .. }, Some(find), Some(place)) if *of > 1 => {
                    Some((&find.text, place))
                }
                _ => None,
            };
            html.push_str(&format!(
                "<p class=\"said\">{}",
                escaped(&phrase::said(&Said::from(landed)))
            ));
            if let Some((find, place)) = steps {
                for (forward, (mark, said)) in
                    [(false, phrase::MATCH_BEFORE), (true, phrase::MATCH_AFTER)]
                {
                    html.push_str(&format!(
                        " <a class=\"step\" href=\"{}\" aria-label=\"{}\">{}</a>",
                        escaped(&searched.stepping(find, place, forward)),
                        escaped(said),
                        escaped(mark)
                    ));
                }
            }
            html.push_str("</p>");
        }
        for notice in &notices {
            html.push_str(&format!(
                "<p class=\"notice\">{}</p>",
                escaped(&format!("{WARNING} {}", phrase::notice(notice)))
            ));
        }
        if let Some(count) = to_check {
            html.push_str(&format!(
                "<p class=\"notice\">{}<a href=\"{}\">{}</a></p>",
                escaped(&format!(
                    "{WARNING} {}",
                    phrase::to_check_in_these_trees(count)
                )),
                escaped(&self.asked.listing(None)),
                escaped(phrase::show_the_trees())
            ));
        }
        html.push_str("</footer>");
    }
}

/// The beads that `blocker` names among what blocks `node`.
///
/// A blocker is named by its id alone. It stands for each dependency of that
/// id over a blocking edge to an unfinished bead, in whichever project, so
/// where several projects hold one, each is named. Where none is, it is the
/// dependency of that id in `node`'s own project, and otherwise the bead of
/// that id in `node`'s own tracker.
fn blocking(snapshot: &Snapshot, node: &Node, blocker: &str) -> Vec<Related> {
    let named = || {
        node.depends_on
            .iter()
            .filter(|related| related.id == blocker)
    };
    let blocks: Vec<Related> = named()
        .filter(|related| {
            related.edge == Edge::Blocks
                && !related.status.as_ref().is_some_and(Status::is_finished)
        })
        .cloned()
        .collect();
    if !blocks.is_empty() {
        return blocks;
    }
    if let Some(own) = named().find(|related| related.project.is_none()) {
        return vec![own.clone()];
    }
    let key = BeadKey {
        project: node.project.clone(),
        id: blocker.to_string(),
    };
    snapshot
        .node(&key)
        .map(|own| Related {
            id: own.id.clone(),
            project: None,
            edge: Edge::Blocks,
            status: Some(own.status.clone()),
            title: Some(own.title.clone()),
        })
        .into_iter()
        .collect()
}

/// Spans of one block as HTML, each in its own style, the first of them the
/// block's span `from`. One that `marks` names as a link is a link, where it
/// points somewhere a page may link to, and one it names as a literal is in
/// its colour as written. One in the code tone is marked as code.
fn spans(html: &mut String, spans: &[Span<'static>], block: Block, marks: &[Mark], from: usize) {
    for (at, span) in spans.iter().enumerate() {
        let code = if span.style.fg == palette::CODE.fg {
            " class=\"code\""
        } else {
            ""
        };
        let mut marked = marks
            .iter()
            .filter(|mark| mark.block == block && mark.at == from + at);
        let css = if marked
            .clone()
            .any(|mark| matches!(mark.is, Marked::Literal))
        {
            palette::css_as_written(span.style)
        } else {
            palette::css(span.style)
        };
        let style = format!("{code}{}", styled(css));
        let text = escaped(&span.content);
        let to = marked
            .find_map(Mark::link)
            .filter(|to| to.starts_with("https://") || to.starts_with("http://"));
        match to {
            Some(to) => html.push_str(&format!("<a href=\"{}\"{style}>{text}</a>", escaped(to))),
            None if style.is_empty() => html.push_str(&text),
            None => html.push_str(&format!("<span{style}>{text}</span>")),
        }
    }
}

/// How many levels a phone indents a line by before it stops. A line deeper
/// than that is drawn at that depth, and drawing its tree as the only root
/// is the way further down.
const PHONE_LEVELS: usize = 5;

/// The connectors a line hangs under, with each level a phone draws as a
/// rail in an element of its own: a trunk as `i`, a gap as `b`, and the
/// line's elbow as `u`, classed by whether it is the last line hung there
/// and whether it hangs on a blocking edge. Under the trees the outermost
/// level is the column a project's roots hang in, which a phone draws no
/// rail for, and a level past what a phone indents is left as text. A
/// prefix that is no line's connectors, such as a header's marker, is `None`.
fn railed(prefix: &str, listed: bool) -> Option<String> {
    let (indent, levels) = lines::levels(prefix)?;
    let unrailed = usize::from(!listed);
    let elbow = levels.len() - 1;
    let mut html = escaped(indent);
    for (at, level) in levels.iter().enumerate() {
        let drawn = escaped(level.drawn());
        let railed = at >= unrailed && (at == elbow || at - unrailed < PHONE_LEVELS - 1);
        let (tag, class) = match level {
            _ if !railed => {
                html.push_str(&drawn);
                continue;
            }
            Level::Trunk => ("i", String::new()),
            Level::Gap => ("b", String::new()),
            Level::Elbow {
                last, blocks, up, ..
            } => {
                let classes: Vec<_> = [(last, "last"), (blocks, "blocks"), (up, "up")]
                    .into_iter()
                    .filter_map(|(on, class)| on.then_some(class))
                    .collect();
                let class = if classes.is_empty() {
                    String::new()
                } else {
                    format!(" class=\"{}\"", classes.join(" "))
                };
                ("u", class)
            }
        };
        html.push_str(&format!("<{tag}{class}>{drawn}</{tag}>"));
    }
    Some(html)
}

/// The class of the mark the page draws for `status` in place of its glyph,
/// where it draws one. Any other status keeps its glyph.
fn drawn_mark(status: &Status) -> Option<&'static str> {
    match status {
        Status::Open => Some("open"),
        Status::InProgress => Some("in-progress"),
        Status::Blocked => Some("blocked"),
        Status::Closed => Some("closed"),
        Status::Deferred => Some("deferred"),
        Status::Pinned | Status::Hooked | Status::Other(_) => None,
    }
}

/// The attribute marking the entry or switch that is in force, where `on`.
fn on(on: bool) -> &'static str {
    if on {
        " class=\"on\""
    } else {
        ""
    }
}

/// A style attribute holding `css`, or nothing where it is empty.
fn styled(css: String) -> String {
    if css.is_empty() {
        css
    } else {
        format!(" style=\"{css}\"")
    }
}

/// `text` safe to write between tags and inside a quoted attribute.
fn escaped(text: &str) -> String {
    let mut said = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => said.push_str("&amp;"),
            '<' => said.push_str("&lt;"),
            '>' => said.push_str("&gt;"),
            '"' => said.push_str("&quot;"),
            '\'' => said.push_str("&#39;"),
            c => said.push(c),
        }
    }
    said
}

/// The page's look: its type, and how the terminal's grid is laid out on it.
/// Its colours are `palette::page_colours`. The type is three faces:
/// `--body` for the page, `--display` for a heading, and `--code` for code
/// and for the tree's box-drawing, whose columns line up only in a fixed
/// pitch. Each names its face first and falls back to the reader's own.
const STYLE: &str = "\
:root{--body:Inter,system-ui,-apple-system,\"Segoe UI\",sans-serif;\
--display:\"Space Grotesk\",Inter,system-ui,sans-serif;\
--code:\"JetBrains Mono\",ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}\
html{color-scheme:light dark}\
body{margin:0;background:var(--surface);color:var(--fg);font:14px/1.45 var(--body)}\
body{display:flex;flex-direction:column;min-height:100dvh;box-sizing:border-box}main{flex:1 0 auto;align-content:start}\
.line>:has(+.bead),.line>.left>:not(:last-child),.code{font-family:var(--code)}\
.code{font-size:.9em}.line .bead .code{font-size:inherit}\
main{display:grid;grid-template-columns:minmax(0,1fr)}\
main.open{grid-template-columns:minmax(0,3fr) minmax(0,2fr)}\
.forest{padding:8px 0;min-width:0;overflow-x:auto}\
.line{display:flex;white-space:pre;padding:0 12px;box-sizing:border-box;width:max-content;min-width:100%}\
.line .state{margin-left:auto;padding-left:2ch;flex:none}\
.line.shown{background:var(--sel)}\
.line i,.line b,.line u{font:inherit;text-decoration:none}\
a{color:inherit;text-decoration:none}\
a.fold:hover,a.bead:hover .title,a.close:hover{text-decoration:underline}\
.bead .id{font:500 12px var(--body);color:var(--muted);letter-spacing:.01em}\
aside{position:sticky;top:0;align-self:start;max-height:100vh;overflow:auto;box-sizing:border-box;\
padding:8px 12px;border-left:1px solid var(--line)}\
.page{white-space:pre-wrap}\
.page .w{display:none}\
.page .p{display:inline-grid;grid-template-columns:auto minmax(0,1fr);align-items:baseline;\
vertical-align:top;width:100%;white-space:normal}\
.page .lead{white-space:pre}\
.page .p .t{font-size:15px;line-height:1.5;overflow-wrap:anywhere}\
.page .head .title{font-family:var(--display);font-weight:600}\
.close{display:inline-block;margin-bottom:8px}\
.line.heading{align-items:baseline;flex-wrap:wrap;gap:4px 12px;width:auto;padding-block:14px 8px;margin-bottom:4px;\
border-bottom:1px solid var(--line);font-size:12.5px;color:var(--muted)}\
.line.heading:not(:first-child){margin-top:18px}\
.heading .left{min-width:0}.heading h2{white-space:normal;overflow-wrap:anywhere}\
.heading h2{display:inline;margin:0;font:600 15px var(--display);color:var(--fg)}\
.heading .state{display:flex;flex:0 1 auto;flex-wrap:wrap;gap:4px 14px;align-items:baseline;padding-left:0}\
.heading .read,.heading .state>span{white-space:nowrap}\
.mark.ok{color:var(--ok)}.mark.warn,.flag{color:var(--warn)}\
.prog{display:inline-flex;align-items:center;gap:6px}\
.prog .bar{width:44px;height:5px;border-radius:3px;background:var(--line);overflow:hidden}\
.prog .bar>span{display:block;height:100%;background:var(--ok)}\
footer p,.missing{margin:0}\
footer .said{padding:8px 12px;background:var(--surface)}\
footer .notice{display:flex;align-items:center;gap:10px;padding:9px 20px;font-size:13px;\
background:var(--warn-soft);color:var(--warn);border-top:1px solid var(--line)}\
footer .notice a{margin-left:auto;font-weight:600}\
footer .notice a:hover{text-decoration:underline}\
nav{--chevron:url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' fill='none' stroke='black' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'><path d='M4 6l4 4 4-4'/></svg>\");\
--tick:url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' fill='none' stroke='black' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'><path d='M3 8.5l3 3 7-7'/></svg>\");\
--lens:url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' fill='none' stroke='black' stroke-width='1.8' stroke-linecap='round'><circle cx='7' cy='7' r='4.5'/><path d='M10.5 10.5L14 14'/></svg>\");\
display:flex;align-items:center;gap:12px;min-height:56px;box-sizing:border-box;padding:0 20px;\
border-bottom:1px solid var(--line);background:var(--surface)}\
nav .brand{display:flex;align-items:center;gap:8px;margin-right:8px;font:600 17px var(--display);white-space:nowrap}\
nav .brand::before{content:\"\";width:20px;height:20px;box-sizing:border-box;border:2px solid var(--accent);border-radius:50%;\
background:radial-gradient(circle,var(--accent) 3.5px,transparent 4px)}\
.lists summary::after,.lists a::before,nav button::before{content:\"\";flex:none;background:currentColor;\
-webkit-mask:var(--icon) center/contain no-repeat;mask:var(--icon) center/contain no-repeat}\
.lists{position:relative;min-width:0;max-width:280px}\
.lists summary{list-style:none;cursor:pointer;display:flex;align-items:center;gap:8px;height:34px;box-sizing:border-box;\
padding:0 12px;border:1px solid var(--line);border-radius:8px;font-weight:500}\
.lists summary::-webkit-details-marker{display:none}\
.lists summary>span{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}\
.lists summary::after{--icon:var(--chevron);width:13px;height:13px}\
.lists[open] summary{border-color:var(--accent);box-shadow:0 0 0 3px var(--accent-soft)}\
.lists>div{position:absolute;top:calc(100% + 8px);left:0;z-index:5;width:290px;box-sizing:border-box;padding:6px;\
background:var(--raised);border:1px solid var(--line);border-radius:12px;box-shadow:var(--shadow)}\
.lists a{display:flex;align-items:center;gap:10px;padding:9px 10px;border-radius:8px;overflow-wrap:anywhere}\
.lists a::before{--icon:var(--tick);width:16px;height:14px;background:none}\
.lists a.on{background:var(--accent-soft);color:var(--accent);font-weight:600}\
.lists a.on::before{background:currentColor}\
.lists a:hover:not(.on){background:var(--sunk)}\
.lists .count{flex:none;margin-left:auto;font-size:12.5px;font-weight:normal;color:var(--muted)}\
.lists .hd{margin:6px 4px 0;padding:10px 6px 4px;border-top:1px solid var(--line);\
font:600 11px var(--body);letter-spacing:.08em;text-transform:uppercase;color:var(--faint)}\
nav .ready{display:flex;align-items:center;gap:8px;height:34px;padding:0 10px;border-radius:8px;font-weight:500;color:var(--muted)}\
nav .ready::before{content:\"\";flex:none;width:30px;height:18px;border-radius:9px;\
background:radial-gradient(circle at 9px 50%,var(--surface) 7px,transparent 7.5px),var(--rail)}\
nav .ready.on{color:var(--fg)}\
nav .ready.on::before{background:radial-gradient(circle at calc(100% - 9px) 50%,var(--surface) 7px,transparent 7.5px),var(--accent)}\
nav form{flex:0 1 320px;margin-left:auto;display:flex;align-items:center;gap:8px;height:34px;box-sizing:border-box;\
padding:0 12px;border-radius:8px;background:var(--sunk);color:var(--faint)}\
nav button{order:-1;flex:none;display:grid;place-items:center;width:20px;height:20px;padding:0;border:0;border-radius:4px;\
background:none;color:inherit;font-size:0;cursor:pointer}\
nav button::before{--icon:var(--lens);width:16px;height:16px}\
@media (forced-colors:active){nav button{width:auto;font-size:inherit}nav button::before{display:none}\
nav .ready.on{text-decoration:underline}nav form:focus-within{outline:2px solid CanvasText}}\
nav form:focus-within{box-shadow:0 0 0 2px var(--accent)}\
nav label{flex:1;min-width:0;display:flex;align-items:center}\
nav input{flex:1;min-width:0;font:inherit;color:var(--fg);background:none;border:0;padding:0;outline:0}\
nav input::placeholder{color:var(--muted);opacity:1}\
nav label span{display:none}\
nav a:focus-visible,nav summary:focus-visible,nav button:focus-visible{outline:2px solid var(--accent);outline-offset:1px}\
.rooted{display:flex;flex-wrap:wrap;align-items:center;gap:4px 12px;margin:0;padding:10px 20px;\
background:var(--accent-soft);border-bottom:1px solid var(--line);font-size:13.5px}\
.rooted .said{color:var(--muted);min-width:0;overflow-wrap:anywhere}.rooted .title{font-weight:600}.rooted .bead .id{display:none}\
.rooted .bead{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}\
.rooted .all{margin-left:auto;color:var(--accent);font-weight:600;white-space:nowrap}\
.rooted .all::before{content:\"\\2715  \"}a.all:hover{text-decoration:underline}\
.root{display:block;margin-bottom:8px}\
a.root:hover,a.step:hover{text-decoration:underline}\
@media (max-width:720px){main.open{grid-template-columns:minmax(0,1fr)}\
.rooted{padding:9px 14px}.rooted .all::before{content:none}\
main.open:has(.page)>.forest{display:none}\
aside{position:static;max-height:none;border-left:0;padding:8px 16px 24px}\
.line{width:auto;white-space:pre-wrap;position:relative}\
.line:has(>.bead){display:grid;grid-template-columns:auto 1.6em repeat(3,auto) minmax(0,1fr);\
align-items:baseline;white-space:normal;padding-block:6px;border-bottom:1px solid var(--line)}\
.line>:has(+.bead){grid-area:1/1/3/2;align-self:stretch;margin-block:-6px;--arm:16px}\
.line>:has(+.bead),.line>.left>.rails{display:inline-flex;vertical-align:top;font-size:0;white-space:pre}\
.line>.left:has(>.rails){display:inline-flex;max-width:100%}\
.line>.left>.rails+span{min-width:0}\
.line i,.line b,.line u{flex:none;width:14px;position:relative}\
.line i::before,.line u::before,.line u::after{content:\"\";position:absolute;left:10px;\
border:0 solid var(--rail)}\
.line i::before,.line u::before{top:0;bottom:0;border-left-width:1.5px}\
.line u.last::before{bottom:auto;height:var(--arm,9px)}\
.line u::after{top:var(--arm,9px);width:10px;border-top-width:1.5px}\
.line u.blocks::before,.line u.blocks::after{border-style:dashed;border-color:var(--faint)}\
.line>:has(+.bead)::after,.line>.left>.rails::after{content:\"\";flex:none;width:22px;font-size:13px;text-align:center}\
.line>:has(+.bead)::after{padding-top:6px}\
.line>a.fold:has(+.bead)::after,.line>.left>a.fold.rails::after{content:\"▾\"}\
.line>a.fold.shut:has(+.bead)::after,.line>.left>a.fold.shut.rails::after{content:\"▸\"}\
.line>.rails:has(>u.up)::after{content:\"↑\"}\
.line>.bead{grid-area:1/2/3/-1;display:grid;grid:subgrid/subgrid;align-items:baseline}\
.line>.bead::after{content:\"\";position:absolute;inset:0}\
#shown{position:absolute;top:0}\
.line a:not(.bead){position:relative;z-index:1}\
.line .g{grid-area:1/1;font-size:14px;text-align:center}\
.line .title{grid-area:1/2/2/-1;min-width:10em;font-size:15px;line-height:1.35;overflow-wrap:anywhere}\
.line>.bead>.id,.line>.more,.line>.bead~.state{grid-row:2;font-size:12px}\
.line>.bead>.id{grid-column:2}.line>.bead>:not(.title){font-family:var(--body)}\
.line>.more{grid-column:4;padding-left:2ch;overflow-wrap:anywhere}\
.line>.bead~.state{grid-column:5;margin-left:0;overflow-wrap:anywhere}\
nav{order:1;position:sticky;bottom:0;z-index:2;min-height:0;gap:6px;\
padding:8px 10px calc(8px + env(safe-area-inset-bottom));border-bottom:0;border-top:1px solid var(--line);\
font-size:13.5px}\
nav>*,nav>form{flex:1 1 0;min-width:0;margin:0}nav>.lists{flex-grow:1.3;position:static}nav>.brand{display:none}\
.lists summary,nav>.ready,nav form{justify-content:center;gap:6px;height:38px;padding:0 4px;border:0;border-radius:10px;\
background:none;color:var(--muted);font-weight:500;white-space:nowrap;overflow:hidden}\
.lists[open] summary{box-shadow:none}\
nav .ready::before{width:26px;height:16px;\
background:radial-gradient(circle at 8px 50%,var(--surface) 6px,transparent 6.5px),var(--rail)}\
nav .ready.on::before{background:radial-gradient(circle at calc(100% - 8px) 50%,var(--surface) 6px,transparent 6.5px),var(--accent)}\
nav .ready.on,nav:not(:has(.ready.on))>.lists>summary,.lists[open] summary\
{background:var(--accent-soft);color:var(--accent);font-weight:600}\
nav form:focus-within{box-shadow:none}\
nav button{display:none}nav button::before{display:none}\
nav label{justify-content:center;gap:6px;height:100%;cursor:text}\
nav label::before{content:\"\";flex:none;width:16px;height:16px;background:currentColor;\
-webkit-mask:var(--lens) center/contain no-repeat;mask:var(--lens) center/contain no-repeat}\
nav input{flex:none;width:0}nav label span{display:inline}\
nav:has(form:focus-within,input[type=search]:is(:not(:placeholder-shown),:not([value='']))) label span{display:none}\
nav:has(form:focus-within){height:0;padding-block:0;border:0}nav:has(form:focus-within)>:not(form){visibility:hidden}\
nav:has(form:focus-within,input[type=search]:is(:not(:placeholder-shown),:not([value='']))) form{position:fixed;top:0;left:0;right:0;display:flex;gap:8px;height:auto;padding:8px 12px;border-radius:0;\
background:var(--surface);border-bottom:1px solid var(--line)}\
html:has(nav form:focus-within,nav input[type=search]:is(:not(:placeholder-shown),:not([value='']))){scroll-padding-top:52px}body:has(nav form:focus-within,nav input[type=search]:is(:not(:placeholder-shown),:not([value='']))){padding-top:52px}\
nav:has(form:focus-within,input[type=search]:is(:not(:placeholder-shown),:not([value='']))) input{flex:1;width:auto;padding:7px 8px;border-radius:8px;text-align:left;background:var(--sunk);font-size:16px;font-weight:normal}\
footer a.step{display:inline-block;padding:0 10px;font-size:18px}\
footer .notice{padding:9px 16px}\
.line.heading{gap:4px 10px;padding-block:14px 6px;margin:0;border-bottom:0}\
nav:has(form:focus-within,input[type=search]:is(:not(:placeholder-shown),:not([value='']))) button{display:block;order:0;width:auto;height:auto;background:none;color:var(--accent);font-size:16px}\
.lists>div{display:none}\
.lists[open]>div{display:block;position:absolute;top:auto;left:0;right:0;bottom:100%;width:auto;padding:8px 10px 16px;\
max-height:calc(100dvh - 64px);overflow-y:auto;border:0;border-radius:18px 18px 0 0}\
.lists[open]>div::before{content:\"\";display:block;width:38px;height:5px;margin:2px auto 8px;border-radius:3px;background:var(--rail)}\
.lists[open] a{padding:13px 12px;font-size:16px}\
.lists[open]>summary::before{content:\"\";position:fixed;inset:0;background:rgba(8,12,16,.42);cursor:default}\
nav:has(.lists[open]) form{visibility:hidden}\
.page .arrow{display:none}\
.page .bead{display:inline-grid;grid-template-columns:1.6em minmax(0,1fr);vertical-align:top;\
white-space:normal;align-items:baseline;width:calc(100% - 2ch)}\
.page .bead>span:not([class]){display:none}\
.page .bead .g{grid-area:1/1;font-size:14px;text-align:center}\
.page .bead .title{grid-area:1/2;font-size:15px;line-height:1.35;overflow-wrap:anywhere}\
.page .bead .id{grid-area:2/2;font-size:12px;font-family:var(--body)}\
.page .head .bead{width:100%;margin-bottom:4px}\
.page .head .title{font-size:18px}}\
.bead .g:is(.open,.in-progress,.blocked,.closed,.deferred){display:inline-block;position:relative;\
box-sizing:border-box;width:12px;height:12px;border:1.75px solid currentColor;border-radius:50%;\
font-size:0;overflow:hidden;vertical-align:-1px;justify-self:center}\
.bead .g.open{color:var(--status-open)}\
.bead .g.in-progress{background:linear-gradient(90deg,currentColor 50%,transparent 50%)}\
.bead .g.blocked,.bead .g.closed{background:currentColor}\
.bead .g.closed::after{content:\"\";position:absolute;left:2.6px;top:.6px;width:3px;height:6px;\
border:solid var(--surface);border-width:0 1.6px 1.6px 0;transform:rotate(45deg)}\
.bead .g.deferred{border-style:dashed}\
@media print,(forced-colors:active){.bead .g:is(.open,.in-progress,.blocked,.closed,.deferred){\
width:auto;height:auto;border:0;border-radius:0;background:none;font-size:inherit;overflow:visible}\
.bead .g.closed::after{content:none}}";

/// The page's one script, which keeps it current. It asks the page's own
/// address for its changes since the one the page was drawn from, and the
/// server sends them down that one connection: the whole page first, and
/// after it each patch to the page it sent last. Each page replaces the
/// body. It keeps the line or bead view at the top of the window where it
/// was, or the next line below it that is still drawn, knowing a line by its
/// id, and keeps both panes'
/// scroll, the link that had focus, an open menu, and what is typed in the
/// find field. A page differing only in its change is not shown again. A
/// hidden page lets its connection go, and asks again from the change it
/// last heard once it is shown, so pages left in the background hold none of
/// the few connections a browser makes to one server. A page the server is
/// too busy to keep current asks again after as long as a quiet connection
/// waits for a comment. Without it the page is the same page, drawn once.
const KEPT_CURRENT: &str = r##"addEventListener("DOMContentLoaded", () => {
  if (!window.EventSource) return;
  const asked = location.pathname + (location.search ? location.search + "&" : "?");
  let since = document.documentElement.dataset.change, changes, page, shown;
  const heard = (event, lines) => {
    since = event.lastEventId;
    show(page = lines);
  };
  const listen = () => {
    const source = changes = new EventSource(asked + "since=" + since);
    source.addEventListener("error", () => {
      if (source.readyState !== EventSource.CLOSED || changes !== source) return;
      changes = null;
      setTimeout(() => document.hidden || changes || listen(), 15000);
    });
    changes.addEventListener("page", (event) => heard(event, event.data.split("\n")));
    changes.addEventListener("patch", (event) => {
      const lines = event.data.split("\n");
      for (let at = 0; at < lines.length; ) {
        const [from, going, coming] = lines[at++].split(" ").map(Number);
        page.splice(from, going, ...lines.slice(at, at += coming));
      }
      heard(event, page);
    });
  };
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) return changes || listen();
    changes?.close();
    changes = null;
  });
  if (!document.hidden) listen();
  const goes = (link) => {
    if (link.origin !== location.origin) return [link.className, link.href].join(" ");
    const where = link.matches(".fold") ? "" : new URLSearchParams(link.search).get("at");
    return [link.className, link.pathname, where, link.hash].join(" ");
  };
  const marks = (on = document) =>
    [...on.querySelectorAll(".line[id]"), on.querySelector("aside")].filter(Boolean);
  const mark = (at) => at.matches("aside") ? "aside" : at.id;
  const boxes = () => [document.querySelector(".forest"), document.querySelector("aside")];
  const pane = (link) => boxes().findIndex((box) => box?.contains(link));
  const alike = (at, kind) => [...(boxes()[at] ?? document).querySelectorAll("a[href]")]
    .filter((link) => goes(link) === kind);
  const show = (page) => {
    const text = page.join("\n").replace(/ data-change="\d+"/, "");
    if (text === shown) return;
    shown = text;
    const drawn = new DOMParser().parseFromString(text, "text/html");
    const coming = new Set(marks(drawn).map(mark));
    const before = marks();
    const top = before.findIndex((at) => at.getBoundingClientRect().bottom > 0);
    const held = top < 0 ? null : before.slice(top).find((at) => coming.has(mark(at)));
    const kept = held && [mark(held), held.getBoundingClientRect().top];
    const focus = document.activeElement?.matches("a[href]") && document.activeElement;
    const focused = focus && [pane(focus), goes(focus)];
    if (focused) focused.push(alike(...focused).indexOf(focus));
    const scrolled = boxes().map((box) => box && [box.scrollLeft, box.scrollTop]);
    const opened = [...document.querySelectorAll("details[open]")].map((menu) => menu.className);
    const summary = document.activeElement?.matches("summary") && document.activeElement.parentElement.className;
    const field = document.querySelector("input[type=search]");
    const typed = field && field.value !== field.defaultValue ? field.value : null;
    const caret = field && field === document.activeElement
      && [field.selectionStart, field.selectionEnd, field.selectionDirection];
    document.title = drawn.title;
    document.body.replaceWith(drawn.body);
    const menus = [...document.querySelectorAll("details")];
    menus.forEach((menu) => menu.open ||= opened.includes(menu.className));
    const menu = summary ? menus.find((menu) => menu.className === summary) : null;
    menu?.querySelector("summary")?.focus({ preventScroll: true });
    const typing = document.querySelector("input[type=search]");
    if (typing && typed !== null) typing.value = typed;
    if (typing && caret) {
      typing.focus({ preventScroll: true });
      typing.setSelectionRange(...caret);
    }
    const again = kept && marks().find((at) => mark(at) === kept[0]);
    if (again) scrollBy(0, again.getBoundingClientRect().top - kept[1]);
    boxes().forEach((box, at) => {
      if (box && scrolled[at]) [box.scrollLeft, box.scrollTop] = scrolled[at];
    });
    if (focused) alike(focused[0], focused[1])[focused[2]]?.focus({ preventScroll: true });
  };
});"##;

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::model::anomaly::Anomaly;
    use crate::model::snapshot::{ProviderState, Tree};
    use crate::model::tree::Link;
    use crate::model::types::{Edge, Status};
    use crate::view::address::{address, addressable};
    use crate::view::draw::tests::{counts, drawn_at, frame_of, node, snapshot, tree};
    use crate::view::forest::{flatten, List};
    use crate::view::Action;

    const PROJECT: &str = "summit-works";

    fn key(id: &str) -> BeadKey {
        BeadKey {
            project: PROJECT.to_string(),
            id: id.to_string(),
        }
    }

    fn related(bead: &Node, edge: Edge) -> Related {
        Related {
            id: bead.id.clone(),
            project: None,
            edge,
            status: Some(bead.status.clone()),
            title: Some(bead.title.clone()),
        }
    }

    /// One invented tree with something of every shape the page draws: a
    /// root resting open over ready work, a branch resting open beneath it, a
    /// bead shut over a child of its own, a run of finished siblings, and a
    /// bead blocked by another.
    fn ground() -> Snapshot {
        let mut beads = vec![
            node("smt-4kd3p", "lift the ground station", Status::InProgress),
            node("smt-4kd3p.1", "wire the niri theme include", Status::Open),
            node("smt-4kd3p.4", "restore app theming", Status::Open),
            node("smt-4kd3p.2", "retire the DMS remnants", Status::Closed),
            node("smt-4kd3p.3", "land the session shell", Status::Closed),
            node("smt-4kd3p.5", "make the switch permanent", Status::Closed),
            node("smt-4kd3p.6", "apply the two niri settings", Status::Open),
            node("smt-4kd3p.7", "confirm the wedges are gone", Status::Open),
            node("smt-4kd3p.8", "read the wedge log", Status::Open),
        ];
        beads[2].ready = true;
        let wire = related(&beads[1], Edge::Blocks);
        beads[6].blocked_by = vec![wire.id.clone()];
        beads[6].depends_on = vec![wire];
        let apply = related(&beads[6], Edge::Blocks);
        beads[1].blocks = vec![apply];
        let root = related(&beads[0], Edge::ParentChild);
        for bead in &mut beads[1..] {
            bead.parent = Some(root.clone());
        }
        let link = |bead| Link {
            bead,
            edge: Edge::ParentChild,
            first: true,
        };
        let children = vec![
            vec![link(1), link(3), link(4), link(5), link(6)],
            vec![link(2)],
            vec![],
            vec![],
            vec![],
            vec![],
            vec![link(7)],
            vec![link(8)],
            vec![],
        ];
        for (view, listed) in VIEWS {
            for &at in *listed {
                beads[at].views.push((*view).to_string());
            }
        }
        let total = beads.len();
        let mut ground = snapshot(
            vec![Tree {
                beads,
                children,
                counts: counts(3, total, 0, 0),
                ..tree(
                    PROJECT,
                    "smt-4kd3p",
                    "lift the ground station",
                    counts(0, 0, 0, 0),
                )
            }],
            Vec::new(),
            ProviderState::Absent,
        );
        ground.views = VIEWS.iter().map(|(view, _)| (*view).to_string()).collect();
        ground
    }

    /// The views the ground's config names, each with the beads it lists:
    /// one of beads waiting on other work, and one holding the ready bead.
    const VIEWS: &[(&str, &[usize])] = &[("theming", &[1, 6]), ("settings", &[2, 6, 7])];

    /// The page is drawn at the terminal's instant, an hour east of UTC, so
    /// a time of day said in UTC would be an hour out.
    fn drawing() -> Drawn {
        Drawn {
            at: drawn_at(),
            offset: FixedOffset::east_opt(3600).expect("an offset"),
            collecting: Vec::new(),
            change: 7,
        }
    }

    /// What the terminal says of the project's read, half a minute old, and
    /// what the page says in its place.
    const AGO: &str = "✓ 30s ago";
    const READ_AT: &str = "✓ read at 11:21";

    fn page_at(target: &str) -> Answer {
        let asked = Asked::of(target).expect("a page is at the address");
        page(ground(), &Layout::default(), &asked, drawing())
    }

    /// What a reader sees of some HTML: its text, tags taken out and
    /// entities read back.
    fn text_of(html: &str) -> String {
        let mut text = String::new();
        let mut in_tag = false;
        for c in html.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => text.push(c),
                _ => {}
            }
        }
        text.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&amp;", "&")
    }

    /// Words apart by any run of blank columns, as the same words are apart
    /// on a page and on a terminal that pads them to a column.
    fn words(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Each line of the forest on the page, whole from its opening tag.
    fn lines_html(html: &str) -> Vec<&str> {
        html.split("<div class=\"line")
            .skip(1)
            .map(|line| &line[..line.find("</div>").expect("the line ends")])
            .collect()
    }

    /// The HTML inside one line of the forest on the page.
    fn line_html(html: &str, at: usize) -> &str {
        let line = lines_html(html)[at];
        &line[line.find('>').expect("the tag ends") + 1..]
    }

    /// The id a line of the forest carries, where it carries one.
    fn line_id(html: &str, at: usize) -> Option<String> {
        let line = lines_html(html)[at];
        let tag = &line[..line.find('>').expect("the tag ends")];
        let id = tag.split(" id=\"").nth(1)?;
        Some(id[..id.find('"').expect("the id ends")].replace("&amp;", "&"))
    }

    /// The line of the page a browser lands on for `fragment`.
    fn landed_on(html: &str, fragment: &str) -> usize {
        (0..lines_html(html).len())
            .find(|&at| line_id(html, at).as_deref() == Some(fragment))
            .expect("a line carries the fragment")
    }

    /// Each line of the forest on the page, as a reader sees it, with the
    /// state block and a heading's read apart from the rest as the terminal
    /// sets them apart.
    fn page_lines(html: &str) -> Vec<String> {
        (0..lines_html(html).len())
            .map(|at| {
                let line = line_html(html, at)
                    .replace("<span class=\"state\">", " ")
                    .replace("<span class=\"read\">", " ")
                    .replace("<span class=\"flag\">", " ");
                words(&text_of(&in_the_terminals_order(&line)))
            })
            .collect()
    }

    /// `html` with each reference's id moved from after its title to before
    /// it, which is where the terminal draws an id.
    fn in_the_terminals_order(html: &str) -> String {
        let mut html = html.to_string();
        let mut from = 0;
        while let Some(found) = html[from..].find("<span class=\"g") {
            let mark = from + found;
            let after_mark = element_end(&html, mark);
            let id = after_mark
                + html[after_mark..]
                    .find("<span class=\"id\">")
                    .expect("an id");
            let after_id = element_end(&html, id);
            let moved = html[id..after_id].to_string();
            html.replace_range(id..after_id, "");
            html.insert_str(after_mark, &format!(" {moved}"));
            from = after_id;
        }
        html
    }

    /// Each line of the forest the terminal draws, as a reader sees it, with
    /// the project's line said as the page's heading says it: the read dated
    /// and the fraction done in words.
    fn terminal_lines(forest: &Forest) -> Vec<String> {
        let rows = frame_of(forest, 160, 60).rows();
        rows[..forest.lines().len()]
            .iter()
            .map(|row| {
                let row = words(row);
                if !row.contains(AGO) {
                    return row;
                }
                row.replace(AGO, READ_AT)
                    .split(' ')
                    .map(|word| match word.split_once('/') {
                        Some((done, of)) => format!("{done} of {of} done"),
                        None => word.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    /// The forest the terminal draws for the ground, with the folds on the
    /// lines `toggled` names toggled by key, in that order.
    fn terminal_forest(toggled: &[usize]) -> Forest {
        let mut forest = flatten(ground());
        for &at in toggled {
            forest.select_line(at);
            forest.apply(Action::ToggleFold);
        }
        forest
    }

    /// What a browser asks for when the fold link on line `at` of a page is
    /// followed: the link without its fragment, which stays in the browser.
    fn fold_link(html: &str, at: usize) -> String {
        let (asked, fragment) = fold_link_whole(html, at);
        assert_eq!(landed_on(html, &fragment), at);
        asked
    }

    /// The fold link on line `at` of a page, as what a browser asks for and
    /// the fragment it keeps.
    fn fold_link_whole(html: &str, at: usize) -> (String, String) {
        let line = line_html(html, at);
        let link = line
            .split("<a class=\"fold")
            .nth(1)
            .and_then(|link| link.split_once(" href=\""))
            .expect("the line folds")
            .1;
        let link = link[..link.find('"').expect("the link ends")].replace("&amp;", "&");
        let (asked, fragment) = link.split_once('#').expect("the link lands on its line");
        (asked.to_string(), fragment.to_string())
    }

    /// The ground with a bead more under its root, drawn above every other
    /// child of it.
    fn with_a_bead_above() -> Snapshot {
        let mut snapshot = ground();
        let tree = std::sync::Arc::make_mut(&mut snapshot.trees[0]);
        let mut bead = node("smt-4kd3p.9", "chart the new ground", Status::Open);
        bead.parent = tree.beads[1].parent.clone();
        tree.beads.push(bead);
        tree.children.push(vec![]);
        tree.children[0].insert(
            0,
            Link {
                bead: tree.beads.len() - 1,
                edge: Edge::ParentChild,
                first: true,
            },
        );
        snapshot.collected = snapshot.trees.clone();
        snapshot
    }

    #[test]
    fn a_fold_link_lands_on_its_own_line_after_a_bead_appears_above_it() {
        let before = page_at("/").html;
        let shut = lines_titled(&before, "apply the two niri settings")[0];
        let (asked, fragment) = fold_link_whole(&before, shut);

        let after = page_of(with_a_bead_above(), &asked);
        assert!(text_of(&after).contains("chart the new ground"));
        let landed = landed_on(&after, &fragment);
        assert_ne!(landed, shut, "the bead above moved the line down");
        assert!(
            page_lines(&after)[landed].contains("apply the two niri settings"),
            "{:#?}",
            page_lines(&after)
        );
    }

    /// A browser takes `:~:` in a fragment for the start of a directive and
    /// looks for no element by what follows, which a project named `~` would
    /// put in the anchor of every bead's line.
    #[test]
    fn no_anchor_holds_what_a_browser_takes_for_a_fragment_directive() {
        let mut ground = ground();
        for tree in ground.trees.iter_mut().chain(ground.collected.iter_mut()) {
            let tree = std::sync::Arc::make_mut(tree);
            tree.project = "~".to_string();
            for bead in &mut tree.beads {
                bead.project = "~".to_string();
            }
        }
        ground.projects = vec!["~".to_string()];
        let html = page_of(ground, "/");
        assert!(html.contains("class=\"fold"));
        assert!(!html.contains(":~:"), "{html}");
    }

    /// A link lands on a line by its id, so a line no link lands on, one
    /// that neither folds nor is a bead's row, carries none.
    #[test]
    fn a_line_that_neither_folds_nor_is_a_beads_row_carries_no_id() {
        let html = page_at("/").html;
        let lines = lines_html(&html);
        let folds = |line: &&str| line.contains("class=\"fold");
        let bead = |line: &&str| line.contains("class=\"bead\"");
        assert!(lines.iter().any(folds));
        assert!(lines.iter().any(|line| !folds(line) && bead(line)));
        assert!(lines.iter().any(|line| !bead(line)));
        for (at, line) in lines.iter().enumerate() {
            assert_eq!(
                line_id(&html, at).is_some(),
                folds(line) || bead(line),
                "{line}"
            );
        }
    }

    #[test]
    fn the_page_draws_every_line_of_the_forest_as_the_terminal_draws_it() {
        let html = page_at("/").html;
        assert_eq!(page_lines(&html), terminal_lines(&terminal_forest(&[])));
    }

    /// A page left open says the same thing for as long as it is open, so
    /// it says when the project was read rather than how long ago, behind
    /// the same mark, and says how far through it is in words beside a bar.
    #[test]
    fn a_projects_heading_says_the_time_of_day_it_was_read_where_the_terminal_says_how_long_ago() {
        let terminal = frame_of(&terminal_forest(&[]), 160, 60).rows();
        assert_eq!(words(&terminal[0]), format!("▾ summit-works {AGO} 3/9"));
        let html = page_at("/").html;
        assert_eq!(
            page_lines(&html)[0],
            format!("▾ summit-works {READ_AT} 3 of 9 done")
        );
        assert!(lines_html(&html)[0].starts_with(" heading\""), "{html}");
        let heading = line_html(&html, 0);
        assert!(heading.contains("<h2>summit-works</h2>"), "{heading}");
        assert!(
            heading.contains("<span class=\"mark ok\">✓</span>"),
            "{heading}"
        );
        assert!(
            heading.contains("<span style=\"width:33%\"></span>"),
            "{heading}"
        );
    }

    /// The short time of day keeps the whole of the read, day and zone
    /// included, for a reader whose page has been open since another day.
    #[test]
    fn a_headings_read_time_holds_the_whole_date_and_zone() {
        let html = page_at("/").html;
        let heading = line_html(&html, 0);
        let time = &heading[heading.find("<time ").expect("a time")..];
        let full = time
            .split("title=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("a title");
        assert!(full.starts_with("read at 11:21:"), "{full}");
        assert!(full.contains(" on "), "{full}");
        assert!(time.starts_with("<time datetime=\""), "{time}");
    }

    /// A page drawn on another day than the read says which day it was.
    #[test]
    fn a_heading_read_on_another_day_than_the_page_is_drawn_says_the_day() {
        let drawn = Drawn {
            at: drawn_at() + chrono::TimeDelta::days(1),
            ..drawing()
        };
        let asked = Asked::of("/").expect("the forest");
        let html = page(ground(), &Layout::default(), &asked, drawn).html;
        assert!(
            page_lines(&html)[0].contains("read at 11:21 on 30 Aug"),
            "{}",
            page_lines(&html)[0]
        );
    }

    /// The beads in a project to check are said in its heading, in the
    /// warning's colour.
    #[test]
    fn a_heading_says_the_beads_in_the_project_to_check() {
        let checked = with_the_bead_changed(|node| {
            node.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        });
        let asked = Asked::of("/").expect("the forest");
        let html = page(checked, &Layout::default(), &asked, drawing()).html;
        assert_eq!(
            page_lines(&html)[0],
            format!("▾ summit-works {READ_AT} 3 of 9 done {WARNING} 1 bead to check")
        );
        assert!(
            line_html(&html, 0).contains(&format!(
                "<span class=\"flag\">{WARNING} 1 bead to check</span>"
            )),
            "{}",
            line_html(&html, 0)
        );
    }

    /// A read gone unanswered wears the warning's colour on its mark.
    #[test]
    fn a_heading_whose_read_went_badly_marks_it_in_the_warnings_colour() {
        let drawn = Drawn {
            collecting: vec![Awaited {
                wanted: crate::app::Wanted::Everything,
                asked_at: drawn_at() - chrono::TimeDelta::seconds(60),
                patience: chrono::TimeDelta::seconds(30),
            }],
            ..drawing()
        };
        let asked = Asked::of("/").expect("the forest");
        let html = page(ground(), &Layout::default(), &asked, drawn).html;
        assert!(
            line_html(&html, 0).contains("<span class=\"mark warn\">"),
            "{}",
            line_html(&html, 0)
        );
    }

    /// The beads to check that the ready list leaves out are said once, in
    /// a bar at the foot rather than a line of the list, and its link draws
    /// the trees, whose rows say what is wrong with them.
    #[test]
    fn the_beads_to_check_under_a_list_are_a_bar_at_the_foot_that_links_to_their_trees() {
        let checked = || {
            with_the_bead_changed(|node| {
                node.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
            })
        };
        let asked = Asked::of(&link_saying(&page_at("/").html, phrase::ready_only()))
            .expect("the ready list");
        let ready = page(checked(), &Layout::default(), &asked, drawing()).html;
        let said = phrase::to_check_in_these_trees(1);
        assert!(
            !page_lines(&ready)
                .iter()
                .any(|line| line.contains(&phrase::to_check_in_the_forest(1))),
            "{:#?}",
            page_lines(&ready)
        );
        let footer = &ready[ready.find("<footer>").expect("a foot")..];
        assert!(
            footer.contains(&format!("<p class=\"notice\">{WARNING} {said}<a href=")),
            "{footer}"
        );

        let trees = Asked::of(&link_saying(footer, phrase::show_the_trees())).expect("the trees");
        let trees = page(checked(), &Layout::default(), &trees, drawing()).html;
        assert!(!trees.contains("<a class=\"ready on\""), "{trees}");
        assert!(!trees.contains("<footer>"), "{trees}");
    }

    /// A read out for longer than it may be is drawn as the terminal draws
    /// it, over the rows the last read left.
    #[test]
    fn a_read_that_has_gone_unanswered_is_said_over_the_last_rows_read() {
        let drawn = Drawn {
            collecting: vec![Awaited {
                wanted: crate::app::Wanted::Everything,
                asked_at: drawn_at() - chrono::TimeDelta::seconds(60),
                patience: chrono::TimeDelta::seconds(30),
            }],
            ..drawing()
        };
        let asked = Asked::of("/").expect("the forest");
        let lines = page_lines(&page(ground(), &Layout::default(), &asked, drawn).html);
        assert!(lines[0].starts_with("▾ summit-works ⠿"), "{}", lines[0]);
        assert!(lines
            .iter()
            .any(|line| line.contains("apply the two niri settings")));
    }

    #[test]
    fn a_fold_link_draws_the_forest_as_the_terminal_draws_it_with_that_fold_toggled() {
        let lines = terminal_lines(&terminal_forest(&[]));
        let shut = lines
            .iter()
            .position(|line| line.contains("▸ ○ .6"))
            .expect("the blocked bead rests shut over its child");

        let opened = page_at(&fold_link(&page_at("/").html, shut)).html;
        let toggled = terminal_lines(&terminal_forest(&[shut]));
        assert!(
            toggled
                .iter()
                .any(|line| line.contains("confirm the wedges are gone")),
            "{toggled:#?}"
        );
        assert_eq!(page_lines(&opened), toggled);

        let shut_again = page_at(&fold_link(&opened, shut)).html;
        assert_eq!(page_lines(&shut_again), lines);
    }

    /// A fold pointed under a line that was then shut is still pointed once
    /// that line is opened again, as it is in the terminal.
    #[test]
    fn a_fold_pointed_under_a_shut_line_is_still_pointed_when_the_line_opens_again() {
        let lines = terminal_lines(&terminal_forest(&[]));
        let parent = lines
            .iter()
            .position(|line| line.contains("▸ ○ .6"))
            .expect("the blocked bead rests shut over its child");
        let child = parent + 1;

        let opened = page_at(&fold_link(&page_at("/").html, parent)).html;
        let child_shut = page_at(&fold_link(&opened, child)).html;
        let parent_shut = page_at(&fold_link(&child_shut, parent)).html;
        let reopened = page_at(&fold_link(&parent_shut, parent)).html;

        let terminal = terminal_lines(&terminal_forest(&[parent, child, parent, parent]));
        assert!(
            terminal
                .iter()
                .any(|line| line.contains("read the wedge log")),
            "{terminal:#?}"
        );
        assert_eq!(page_lines(&reopened), terminal);
    }

    /// Going to a bead opens the lines over it, and a fold the reader pointed
    /// on a line that brings into sight is as they pointed it.
    #[test]
    fn a_fold_pointed_under_a_shut_line_is_still_pointed_when_going_to_a_bead_opens_the_line() {
        let parent = terminal_lines(&terminal_forest(&[]))
            .iter()
            .position(|line| line.contains("▸ ○ .6"))
            .expect("the blocked bead rests shut over its child");
        let child = parent + 1;

        let opened = page_at(&fold_link(&page_at("/").html, parent)).html;
        let child_opened = fold_link(&opened, child);
        let parent_shut = fold_link(&page_at(&child_opened).html, parent);
        let to_the_child = crate::view::address::Asked::of(&parent_shut)
            .expect("an address")
            .opening(&key("smt-4kd3p.7"));
        let to_the_child = to_the_child.split_once('#').expect("a fragment").0;

        let mut terminal = terminal_forest(&[parent, child, parent]);
        assert!(terminal.go_to(&key("smt-4kd3p.7")));
        let terminal = terminal_lines(&terminal);
        assert!(
            terminal
                .iter()
                .any(|line| line.contains("read the wedge log")),
            "{terminal:#?}"
        );
        assert_eq!(page_lines(&page_at(to_the_child).html), terminal);
    }

    /// The terminal points a fold on the line it has selected, and its bead
    /// view is over the selection, so shutting a line over the shown bead
    /// shows the bead on that line, with its fold shut.
    #[test]
    fn shutting_a_fold_over_the_shown_bead_shows_the_bead_on_that_line() {
        let shown = page_at(&address(&key("smt-4kd3p.7"))).html;
        let mut terminal = flatten(ground());
        assert!(terminal.go_to(&key("smt-4kd3p.7")));
        let parent = terminal_lines(&terminal)
            .iter()
            .position(|line| line.contains("── ○ .6 "))
            .expect("going to the bead opened its parent");

        let link = fold_link(&shown, parent);
        assert!(link.starts_with("/summit-works/smt-4kd3p.6?"), "{link}");
        let answer = page_at(&link);
        terminal.select_line(parent);
        terminal.apply(Action::ToggleFold);
        assert_eq!(page_lines(&answer.html), terminal_lines(&terminal));
        let aside = &answer.html[answer.html.find("<aside>").expect("the bead view")..];
        assert!(text_of(aside).contains("apply the two niri settings"));
    }

    /// Closing the bead view leaves open what was opened to reach the bead,
    /// as closing the terminal's bead window does.
    #[test]
    fn the_folds_opened_to_reach_a_bead_stay_open_once_its_view_is_closed() {
        let shown = page_at(&address(&key("smt-4kd3p.7"))).html;
        let close = closing_link(&shown);
        let (close, _) = close
            .split_once('#')
            .expect("the link lands on the bead's row");

        let mut terminal = flatten(ground());
        assert!(terminal.go_to(&key("smt-4kd3p.7")));
        assert_eq!(page_lines(&page_at(close).html), terminal_lines(&terminal));
    }

    /// The whole address the bead view's back link goes to, fragment and all.
    fn closing_link(html: &str) -> String {
        let close = html
            .split("<a class=\"close\" href=\"")
            .nth(1)
            .expect("the bead view closes");
        close[..close.find('"').expect("the link ends")].replace("&amp;", "&")
    }

    /// Closing the bead view lands on the bead's own row of the forest, so a
    /// phone, which draws the bead view in the forest's place, comes back to
    /// where the reader was rather than to the top of a long forest.
    #[test]
    fn closing_a_beads_view_lands_on_its_row() {
        let tree = address(&key("smt-4kd3p.8"));
        let listed = format!("{}?view={}", address(&key("smt-4kd3p.7")), VIEWS[1].0);
        let twice = page_of(reached_twice(), &address(&key("smt-4kd3p.6")));
        for (shown, bead) in [
            (page_at(&tree).html, "smt-4kd3p.8"),
            (page_at(&listed).html, "smt-4kd3p.7"),
            (twice, "smt-4kd3p.6"),
        ] {
            let close = closing_link(&shown);
            let (target, landing) = close.split_once('#').expect("the link lands somewhere");
            let closed = if bead == "smt-4kd3p.6" {
                page_of(reached_twice(), target)
            } else {
                page_at(target).html
            };
            let opened_at = lines_html(&shown)
                .iter()
                .position(|line| line.starts_with(" shown\""))
                .expect("a line is shown");
            let landed = landed_on(&closed, landing);
            assert!(
                text_of(line_html(&closed, landed)).contains(bead),
                "{close}"
            );
            assert_eq!(landed, opened_at, "{close}");
        }
    }

    /// A link lands on the first line carrying its fragment, so no two lines
    /// of a page carry one id: not a bead the trees draw twice, and not a
    /// bead a list draws again under each row expanded to what it waits on.
    /// The live page keeps the reader's place by a line's id, so every line
    /// drawing a bead carries one.
    #[test]
    fn no_two_lines_of_a_page_carry_one_id() {
        let mut listed = page_at(&format!("/?view={}", VIEWS[0].0)).html;
        for title in ["wire the niri theme include", "apply the two niri settings"] {
            let row = page_lines(&listed)
                .iter()
                .position(|line| line.contains(title))
                .expect("the bead is listed");
            listed = page_at(&fold_link(&listed, row)).html;
        }
        assert_eq!(lines_titled(&listed, "lift the ground station").len(), 2);
        for html in [page_of(reached_twice(), "/"), listed] {
            for (at, line) in lines_html(&html).iter().enumerate() {
                if line.contains("class=\"bead\"") {
                    assert!(line_id(&html, at).is_some(), "{line}");
                }
            }
            let ids: Vec<String> = (0..lines_html(&html).len())
                .filter_map(|at| line_id(&html, at))
                .collect();
            let unique: std::collections::HashSet<&String> = ids.iter().collect();
            assert_eq!(unique.len(), ids.len(), "{ids:?}");
        }
    }

    #[test]
    fn a_run_of_finished_beads_opens_by_its_link_as_it_does_by_its_key() {
        let lines = terminal_lines(&terminal_forest(&[]));
        let run = lines
            .iter()
            .position(|line| line.contains("✓ 3 more"))
            .expect("the finished siblings are one run");

        let opened = page_at(&fold_link(&page_at("/").html, run)).html;
        assert_eq!(
            page_lines(&opened),
            terminal_lines(&terminal_forest(&[run]))
        );
        assert!(text_of(&opened).contains("land the session shell"));
    }

    /// The ground with `smt-4kd3p.6` reached a second way, under
    /// `smt-4kd3p.4`, so the forest draws it and its child twice.
    fn reached_twice() -> Snapshot {
        let mut snapshot = ground();
        let tree = std::sync::Arc::make_mut(&mut snapshot.trees[0]);
        tree.children[2].push(Link {
            bead: 6,
            edge: Edge::ParentChild,
            first: false,
        });
        snapshot.collected = snapshot.trees.clone();
        snapshot
    }

    fn page_of(snapshot: Snapshot, target: &str) -> String {
        let asked = Asked::of(target).expect("a page is at the address");
        page(snapshot, &Layout::default(), &asked, drawing()).html
    }

    /// The line the page shows its bead on.
    fn shown_line(html: &str) -> usize {
        lines_html(html)
            .iter()
            .position(|line| line.starts_with(" shown\""))
            .expect("a line is shown")
    }

    /// The lines the page draws `title` on.
    fn lines_titled(html: &str, title: &str) -> Vec<usize> {
        page_lines(html)
            .iter()
            .enumerate()
            .filter(|(_, line)| line.contains(title))
            .map(|(at, _)| at)
            .collect()
    }

    /// What a browser asks for when the bead link on line `at` is followed.
    fn bead_link(html: &str, at: usize) -> String {
        let line = line_html(html, at);
        let link = line
            .split("<a class=\"bead\" href=\"")
            .nth(1)
            .expect("the line opens a bead");
        let link = link[..link.find('"').expect("the link ends")].replace("&amp;", "&");
        link.split_once('#')
            .map_or(link.clone(), |(asked, _)| asked.to_string())
    }

    /// The page of `smt-4kd3p.6` in [`reached_twice`], and the line of the
    /// copy it does not show.
    fn the_other_copy() -> (String, usize) {
        let bead = page_of(reached_twice(), "/summit-works/smt-4kd3p.6");
        let copies = lines_titled(&bead, "apply the two niri settings");
        assert_eq!(copies.len(), 2, "{:#?}", page_lines(&bead));
        let shown = shown_line(&bead);
        let other = copies
            .into_iter()
            .find(|&at| at != shown)
            .expect("one copy is shown");
        (bead, other)
    }

    #[test]
    fn a_bead_drawn_twice_opens_on_the_copy_whose_row_was_followed() {
        let (bead, other) = the_other_copy();

        let opened = page_of(reached_twice(), &bead_link(&bead, other));

        assert_eq!(shown_line(&opened), other, "{:#?}", page_lines(&opened));
    }

    #[test]
    fn a_fold_pointed_on_the_other_copy_of_a_bead_lands_on_that_copy() {
        let (bead, other) = the_other_copy();

        let toggled = page_of(reached_twice(), &fold_link(&bead, other));

        assert_eq!(shown_line(&toggled), other, "{:#?}", page_lines(&toggled));
    }

    /// The address is composed from the project and the id alone, and the
    /// page it names shows that bead: its row is the one shown, and its bead
    /// view is beside the forest.
    #[test]
    fn a_beads_address_composed_from_its_project_and_id_shows_that_bead() {
        let target = address(&key("smt-4kd3p.6"));
        assert_eq!(target, "/summit-works/smt-4kd3p.6");

        let answer = page_at(&target);
        assert!(answer.found);
        let shown = answer
            .html
            .split("class=\"line shown\"")
            .nth(1)
            .expect("a line is shown");
        assert!(
            text_of(&shown[..shown.find("</div>").unwrap()])
                .contains("apply the two niri settings"),
            "{shown}"
        );
        let aside = &answer.html[answer.html.find("<aside>").expect("the bead view")..];
        assert!(words(&text_of(aside)).contains("○ apply the two niri settings smt-4kd3p.6"));
    }

    #[test]
    fn a_project_a_browser_would_drop_from_the_path_has_no_address() {
        for project in ["", ".", ".."] {
            assert!(!addressable(project), "{project:?}");
        }
        assert!(addressable("..ark"));
    }

    #[test]
    fn a_project_named_with_any_characters_has_an_address_that_reads_back() {
        let bead = BeadKey {
            project: "the ark/ü?&#".to_string(),
            id: "ark-5.1".to_string(),
        };
        let asked = Asked::of(&address(&bead)).expect("a page is at the address");
        assert_eq!(asked.bead, Some(bead));
    }

    #[test]
    fn an_address_naming_a_bead_the_forest_does_not_draw_says_so_and_is_not_found() {
        let answer = page_at(&address(&key("smt-zzz")));
        assert!(!answer.found);
        assert!(text_of(&answer.html).contains("smt-zzz is in no tree beady-eye draws"));
    }

    /// The text of the page's `<title>`.
    fn title_of(html: &str) -> &str {
        let from = html.find("<title>").expect("the page has a title") + "<title>".len();
        &html[from..from + html[from..].find("</title>").expect("the title closes")]
    }

    #[test]
    fn a_page_is_titled_for_the_product_and_a_bead_page_for_its_bead_too() {
        assert_eq!(title_of(&page_at("/").html), "beady-eye");
        assert_eq!(title_of(&saying("said")), "beady-eye");
        assert_eq!(
            title_of(&page_at(&address(&key("smt-4kd3p.1"))).html),
            "wire the niri theme include · beady-eye"
        );
    }

    #[test]
    fn the_page_names_the_product_first_in_its_nav() {
        let html = page_at("/").html;
        let nav = &html[html.find("<nav>").expect("the page has a nav")..];
        assert!(
            nav.starts_with("<nav><span class=\"brand\">beady-eye</span>"),
            "{nav}"
        );
    }

    #[test]
    fn an_address_that_is_no_page_asks_for_nothing() {
        for target in [
            "/summit-works",
            "/a/b/c",
            "summit-works/smt-4kd3p",
            "/a/b?open=%zz",
            "/a/b?shut=%+1",
        ] {
            assert_eq!(Asked::of(target), None, "{target}");
        }
    }

    /// The bead view on the page is the terminal's, row for row, with every
    /// bead it names drawn as a reference to that bead. The row saying what
    /// the bead is blocked by is the one the page draws differently, because
    /// the terminal names the blocker by its id alone.
    #[test]
    fn the_bead_view_draws_the_terminals_rows_with_every_bead_named_by_a_reference() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        let view = &html[html.find("<div class=\"page\">").expect("the bead view")..];
        let page_rows: Vec<String> = text_of(&in_the_terminals_order(
            &view[..view.find("</aside>").unwrap()],
        ))
        .lines()
        .map(words)
        .collect();

        let mut forest = flatten(ground());
        assert!(forest.go_to(&key("smt-4kd3p.6")));
        let node = show::selected(&forest).expect("the bead");
        let mentioned = show::mentioned(&forest, node);
        let said = show::said(
            node,
            &mentioned,
            show::fraction(&forest),
            PROSE,
            usize::MAX,
            &|related| show::followable(&forest, related),
        );
        let blocked_by = said.blocked_by.expect("the bead says what blocks it");
        let terminal_rows: Vec<String> = said
            .rows
            .iter()
            .map(|row| {
                words(
                    &row.iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>(),
                )
            })
            .collect();

        assert_eq!(page_rows.len(), terminal_rows.len(), "{page_rows:#?}");
        for (at, (page, terminal)) in page_rows.iter().zip(&terminal_rows).enumerate() {
            if at == blocked_by {
                assert_eq!(terminal, "blocked by: smt-4kd3p.1");
                assert_eq!(
                    page,
                    "blocked by: ○ smt-4kd3p.1 wire the niri theme include"
                );
            } else {
                assert_eq!(page, terminal, "row {at}");
            }
        }
        assert!(terminal_rows
            .iter()
            .any(|row| row == "→ ○ smt-4kd3p.1 wire the niri theme include"));
    }

    /// Ids are per tracker, so a bead can depend on two beads of one id. The
    /// one the blocked-by line names is the one that blocks it.
    #[test]
    fn a_blocker_is_named_by_its_blocking_edge_where_another_tracker_holds_its_id() {
        let namesake = Related {
            id: "smt-4kd3p.1".to_string(),
            project: Some("kadath".to_string()),
            edge: Edge::Other("related".to_string()),
            status: Some(Status::Closed),
            title: Some("a namesake in another tracker".to_string()),
        };
        let mut ground = ground();
        for tree in ground.trees.iter_mut().chain(ground.collected.iter_mut()) {
            let tree = std::sync::Arc::make_mut(tree);
            tree.beads[6].depends_on.insert(0, namesake.clone());
        }
        let asked = Asked::of(&address(&key("smt-4kd3p.6"))).expect("an address");
        let html = page(ground, &Layout::default(), &asked, drawing()).html;
        let line = text_of(&html)
            .lines()
            .find(|line| line.contains(phrase::blocked_by_ahead()))
            .map(words)
            .expect("the bead says what blocks it");
        assert_eq!(
            line,
            "blocked by: ○ wire the niri theme include smt-4kd3p.1"
        );
    }

    /// The ground with `smt-4kd3p.6` changed by `change` in every tree.
    fn with_the_bead_changed(change: impl Fn(&mut Node)) -> Snapshot {
        let mut ground = ground();
        for tree in ground.trees.iter_mut().chain(ground.collected.iter_mut()) {
            change(&mut std::sync::Arc::make_mut(tree).beads[6]);
        }
        ground
    }

    /// The words of the bead view's line saying what a bead is blocked by
    /// or that it is ready.
    fn readiness_line(snapshot: Snapshot) -> String {
        let html = page_of(snapshot, &address(&key("smt-4kd3p.6")));
        let aside = &html[html.find("<aside>").expect("the bead view")..];
        text_of(aside)
            .lines()
            .find(|line| line.contains(phrase::blocked_by_ahead()) || line.trim() == "ready")
            .map(words)
            .expect("the bead says whether it is ready")
    }

    #[test]
    fn a_blocker_that_is_no_dependency_of_the_bead_is_still_named_by_its_reference() {
        let snapshot = with_the_bead_changed(|bead| {
            bead.blocked_by.push("smt-4kd3p.8".to_string());
        });
        assert_eq!(
            readiness_line(snapshot),
            "blocked by: ○ wire the niri theme include smt-4kd3p.1, ○ read the wedge log smt-4kd3p.8"
        );
    }

    /// A tracker names its blockers by id alone, so a bead of another
    /// project with the same id is not one of them.
    #[test]
    fn a_blocker_is_never_taken_for_another_projects_bead_with_its_id() {
        let snapshot = with_the_bead_changed(|bead| {
            bead.blocked_by.push("smt-4kd3p.8".to_string());
            bead.depends_on.push(Related {
                id: "smt-4kd3p.8".to_string(),
                project: Some("faraway".to_string()),
                edge: Edge::Other("related".to_string()),
                status: Some(Status::Closed),
                title: Some("a bead of another tracker".to_string()),
            });
        });
        assert_eq!(
            readiness_line(snapshot),
            "blocked by: ○ wire the niri theme include smt-4kd3p.1, ○ read the wedge log smt-4kd3p.8"
        );
    }

    /// A bead of another project that blocks the bead is named among its
    /// blockers by its id alone, and is drawn and linked as that project's
    /// bead.
    #[test]
    fn a_blocker_in_another_project_is_named_by_its_reference_and_opens_it() {
        let mut ground = ground();
        for tree in ground.trees.iter_mut().chain(ground.collected.iter_mut()) {
            let beads = &mut std::sync::Arc::make_mut(tree).beads;
            beads[8].project = "faraway".to_string();
            let elsewhere = Related {
                project: Some("faraway".to_string()),
                ..related(&beads[8], Edge::Blocks)
            };
            beads[6].blocked_by.push(elsewhere.id.clone());
            beads[6].depends_on.push(elsewhere);
        }
        let html = page_of(ground.clone(), &address(&key("smt-4kd3p.6")));
        assert_eq!(
            readiness_line(ground),
            "blocked by: ○ wire the niri theme include smt-4kd3p.1, ○ read the wedge log smt-4kd3p.8"
        );
        let blocked_by = &html[html.find(phrase::blocked_by_ahead()).expect("the blockers")..];
        assert!(
            blocked_by.contains("href=\"/faraway/smt-4kd3p.8"),
            "{blocked_by}"
        );
    }

    /// Where the bead's own project holds a namesake it is tied to by no
    /// blocking edge, the blocker is the bead elsewhere that does block it.
    #[test]
    fn a_blocker_elsewhere_wins_over_a_namesake_that_does_not_block() {
        let mut ground = ground();
        for tree in ground.trees.iter_mut().chain(ground.collected.iter_mut()) {
            let beads = &mut std::sync::Arc::make_mut(tree).beads;
            let namesake = related(&beads[8], Edge::Other("related".to_string()));
            let elsewhere = Related {
                project: Some("faraway".to_string()),
                title: Some("a bead of another tracker".to_string()),
                ..related(&beads[8], Edge::Blocks)
            };
            beads[6].blocked_by.push(elsewhere.id.clone());
            beads[6].depends_on.extend([namesake, elsewhere]);
        }
        assert_eq!(
            readiness_line(ground),
            "blocked by: ○ wire the niri theme include smt-4kd3p.1, ○ a bead of another tracker smt-4kd3p.8"
        );
    }

    /// Two projects can each hold a blocker with the one id, and the id
    /// stands for both.
    #[test]
    fn a_blocker_id_two_projects_hold_names_both_their_beads() {
        let mut ground = ground();
        for tree in ground.trees.iter_mut().chain(ground.collected.iter_mut()) {
            let beads = &mut std::sync::Arc::make_mut(tree).beads;
            let wedge = related(&beads[8], Edge::Blocks);
            let held_by = |project: &str, title: &str| Related {
                project: Some(project.to_string()),
                title: Some(title.to_string()),
                ..wedge.clone()
            };
            let both = [
                held_by("faraway", "a bead of one tracker"),
                held_by("yonder", "a bead of another"),
            ];
            beads[6].blocked_by.push(wedge.id.clone());
            beads[6].depends_on.extend(both);
        }
        assert_eq!(
            readiness_line(ground),
            "blocked by: ○ wire the niri theme include smt-4kd3p.1, ○ a bead of one tracker smt-4kd3p.8, ○ a bead of another smt-4kd3p.8"
        );
    }

    /// A link that named a root's copy goes on naming it once its tracker
    /// stops answering, and the line it lands on draws no bead.
    #[test]
    fn a_bead_whose_tracker_did_not_answer_is_not_found() {
        let unread = snapshot(
            vec![Tree::tracker_unreachable(
                PROJECT,
                "smt-4kd3p",
                crate::model::snapshot::TrackerFailure::Unavailable,
            )],
            Vec::new(),
            ProviderState::Absent,
        );
        let target = "/summit-works/smt-4kd3p?at=summit-works%3Asmt-4kd3p";
        let asked = Asked::of(target).expect("a page is at the address");
        let answer = page(unread, &Layout::default(), &asked, drawing());
        assert!(answer
            .html
            .contains(phrase::not_drawn(&key("smt-4kd3p")).as_str()));
        assert!(!answer.found);
    }

    /// A root that draws no bead hangs in the same column as one that does,
    /// so a phone lays its connectors out as it lays out a bead's, beside
    /// one run of text that may wrap.
    #[test]
    fn a_root_that_draws_no_bead_hangs_as_a_beads_root_does() {
        let unread = snapshot(
            vec![Tree::tracker_unreachable(
                PROJECT,
                "smt-4kd3p",
                crate::model::snapshot::TrackerFailure::Unavailable,
            )],
            Vec::new(),
            ProviderState::Absent,
        );
        let html = page_of(unread, "/");
        let root = line_html(&html, 1);
        assert!(!root.contains("class=\"bead"), "{root}");
        assert_eq!(parts(root).0[0], "span.left", "{root}");
        assert_eq!(
            parts(inside(root, "left")).0,
            ["span.rails", "span"],
            "{root}"
        );
    }

    /// Readiness and blockers are read apart, so a tracker can say both, and
    /// the terminal says ready.
    #[test]
    fn a_bead_said_to_be_ready_and_blocked_is_said_to_be_ready_as_the_terminal_says() {
        let snapshot = with_the_bead_changed(|bead| bead.ready = true);
        assert_eq!(readiness_line(snapshot), "ready");
    }

    /// Every bead on every page is named by the one component: no bead's id
    /// appears outside a reference, and every reference holds a bead's title.
    #[test]
    fn no_page_names_a_bead_by_a_bare_id() {
        let ground = ground();
        let beads: Vec<&Node> = ground
            .collected
            .iter()
            .flat_map(|tree| &tree.beads)
            .collect();
        let mut pages = vec!["/".to_string()];
        pages.extend(beads.iter().map(|bead| address(&key(&bead.id))));
        for target in &pages {
            let html = page_at(target).html;
            let (references, rest) = references_and_the_rest(&html);
            let rest = words(&text_of(&rest));
            for bead in &beads {
                let shown = bead
                    .id
                    .rsplit_once('.')
                    .map_or(bead.id.as_str(), |(_, tail)| tail);
                assert!(
                    !rest.contains(&format!(".{shown} ")) && !rest.contains(&bead.id),
                    "{target} names {} outside a reference: {rest}",
                    bead.id
                );
            }
            assert!(!references.is_empty(), "{target}");
            for reference in &references {
                let said = text_of(reference);
                assert!(
                    beads.iter().any(|bead| said.contains(&bead.title)),
                    "{target} has a reference with no title: {said}"
                );
            }
        }
    }

    /// The elements `html` holds at its top level, each as its tag and its
    /// class, and the text between them.
    fn parts(html: &str) -> (Vec<String>, String) {
        let (mut parts, mut between, mut from) = (Vec::new(), String::new(), 0);
        while let Some(found) = html[from..].find('<') {
            let open = from + found;
            between.push_str(&html[from..open]);
            let tag = &html[open + 1..open + html[open..].find('>').expect("the tag ends")];
            let name = tag.split(' ').next().expect("a tag has a name");
            let class = tag
                .split("class=\"")
                .nth(1)
                .map(|class| &class[..class.find('"').expect("the class ends")]);
            parts.push(class.map_or(name.to_string(), |class| format!("{name}.{class}")));
            from = element_end(html, open);
        }
        between.push_str(&html[from..]);
        (parts, between)
    }

    /// What the element of `class` inside `html` holds.
    fn inside<'a>(html: &'a str, class: &str) -> &'a str {
        let open = html
            .find(&format!("class=\"{class}\""))
            .and_then(|at| html[..at].rfind('<'))
            .expect("the element is drawn");
        let end = element_end(html, open);
        let content = open + html[open..].find('>').expect("the tag ends") + 1;
        &html[content..html[..end].rfind('<').expect("a closing tag")]
    }

    /// A phone's stylesheet draws a bead's line as two lines, the glyph and
    /// title over the id and the rest of the row, with the line it hangs
    /// under beside both. So each of those is an element of its own, and
    /// nothing else sits between them that a layout would have to place.
    #[test]
    fn a_bead_line_holds_apart_each_part_a_phone_lays_out() {
        let html = page_at("/").html;
        let beads: Vec<usize> = (0..html.matches("<div class=\"line").count())
            .filter(|&at| line_html(&html, at).contains("class=\"bead\""))
            .collect();
        assert_eq!(beads, [1, 2, 3, 4]);
        let (leaf, shut) = (3, 4);
        for at in beads {
            let line = line_html(&html, at);
            let (shape, between) = parts(line);
            let hung = match at {
                _ if at == leaf => "span.rails",
                _ if at == shut => "a.fold shut rails",
                _ => "a.fold rails",
            };
            assert_eq!(shape, [hung, "a.bead", "span.state"], "{line}");
            assert_eq!(between, "", "{line}");

            let (reference, between) = parts(inside(line, "bead"));
            assert!(reference[0].starts_with("span.g"), "{line}");
            assert_eq!(reference[1..], ["span.title", "span.id"], "{line}");
            assert_eq!(between.trim(), "", "{line}");
        }
    }

    /// Every reference on a page reads as its mark, its title, then its id.
    /// The mark is labelled with `bd`'s word for the status. The id has no
    /// colour of its own, so a closed bead's id is as quiet as an open one's
    /// and no fainter.
    #[test]
    fn a_reference_draws_its_mark_then_its_title_then_a_quiet_id() {
        let html = page_at(&address(&key("smt-4kd3p.2"))).html;
        let (references, _) = references_and_the_rest(&html);
        let mut statuses = Vec::new();
        for reference in &references {
            let inner = &reference[reference.find('>').expect("the tag ends") + 1
                ..reference.rfind('<').expect("a closing tag")];
            let (shape, _) = parts(inner);
            assert!(shape[0].starts_with("span.g"), "{reference}");
            assert_eq!(shape[1..], ["span.title", "span.id"], "{reference}");
            let mark = &inner[..inner.find('>').expect("the mark's tag ends")];
            let label = mark
                .split("aria-label=\"")
                .nth(1)
                .expect("the mark is labelled");
            statuses.push(label[..label.find('"').expect("the label ends")].to_string());
            let id = &inner[inner.find("class=\"id\"").expect("the id is drawn")..];
            assert!(
                !id[..id.find('>').expect("the id's tag ends")].contains("style="),
                "{reference}"
            );
            assert!(!inside(inner, "id").contains("style="), "{reference}");
        }
        for status in ["in_progress", "open", "closed"] {
            assert!(statuses.iter().any(|said| said == status), "{statuses:?}");
        }
        assert!(html.contains("class=\"g closed\""), "{html}");
    }

    /// The row's cells other than the reference are drawn together, after
    /// it and before the state, where a phone draws them after the id.
    #[test]
    fn a_bead_lines_other_cells_are_drawn_together_after_its_reference() {
        let mut snapshot = ground();
        let tree = std::sync::Arc::make_mut(&mut snapshot.trees[0]);
        tree.beads[1].badges = vec![crate::model::badges::Badged {
            key: "pr".to_string(),
            text: "⇢ #12".to_string(),
            short: None,
            link: Some("https://example.invalid/dunwich/12".to_string()),
            colour: None,
            drawn_on: crate::config::DrawnOn::Own,
        }];
        let html = page_of(snapshot, "/");
        let line = line_html(&html, 2);
        assert_eq!(
            parts(line).0,
            ["a.fold rails", "a.bead", "span.more", "span.state"],
            "{line}"
        );
        let more = inside(line, "more");
        assert!(
            more.contains("href=\"https://example.invalid/dunwich/12\""),
            "{more}"
        );
        assert_eq!(words(&text_of(more)), "⇢ #12");
    }

    /// A badge a config gave a literal colour is drawn in that colour on the
    /// forest and on the bead's page, even where the colour is one of `bd`'s
    /// statuses, while a badge that named the status's slot follows the
    /// page's theme.
    #[test]
    fn a_literal_colour_is_drawn_as_written_and_a_status_slot_as_the_theme() {
        use crate::config::{Colour, DrawnOn, Slot};
        use crate::model::badges::Badged;
        use ratatui::style::Color;

        let badged = |key: &str, colour| Badged {
            key: key.to_string(),
            text: key.to_string(),
            short: None,
            link: None,
            colour: Some(colour),
            drawn_on: DrawnOn::Own,
        };
        let blocked = palette::STATUS_BLOCKED
            .fg
            .expect("bd draws a blocked bead in a colour");
        let mut snapshot = ground();
        let tree = std::sync::Arc::make_mut(&mut snapshot.trees[0]);
        tree.beads[1].badges = vec![
            badged("literal", Colour::Absolute(blocked)),
            badged("slot", Colour::Slot(Slot::StatusBlocked)),
        ];
        let opened = key(&tree.beads[1].id);
        snapshot.collected = snapshot.trees.clone();
        let Color::Rgb(r, g, b) = blocked else {
            panic!("bd's status colours are literal");
        };
        let written = format!("color:#{r:02x}{g:02x}{b:02x}\">literal<");
        let themed = "color:var(--status-blocked)\">slot<";

        let forest = page_of(snapshot.clone(), "/");
        let line = line_html(&forest, 2);
        assert!(line.contains(&written), "{line}");
        assert!(line.contains(themed), "{line}");

        let page = page_of(snapshot, &address(&opened));
        let view = &page[page.find("<div class=\"page\">").expect("the bead view")..];
        assert!(view.contains(&written), "{view}");
        assert!(view.contains(themed), "{view}");
    }

    /// One invented tree deeper than a phone indents: a chain seven levels
    /// under the root with a sibling below it at each of its first two
    /// levels, and a bead drawn again under the one it blocks.
    fn deep() -> Snapshot {
        let titles = [
            "refit the lighthouse",
            "rewire the lamp",
            "strip the old wiring",
            "trace the feed",
            "open the junction box",
            "free the seized screw",
            "soak it in oil",
            "wait a night",
            "test the fog signal",
            "log the burn",
        ];
        let mut beads: Vec<Node> = titles
            .iter()
            .enumerate()
            .map(|(at, title)| node(&format!("smt-9lh.{at}"), title, Status::Open))
            .collect();
        beads[7].ready = true;
        let root = related(&beads[0], Edge::ParentChild);
        for bead in &mut beads[1..] {
            bead.parent = Some(root.clone());
        }
        let log = related(&beads[9], Edge::Blocks);
        beads[1].depends_on = vec![log.clone()];
        beads[1].blocked_by = vec![log.id.clone()];
        let link = |bead, edge, first| Link { bead, edge, first };
        let child = |bead| link(bead, Edge::ParentChild, true);
        let mut children: Vec<Vec<Link>> = vec![vec![]; beads.len()];
        children[0] = vec![child(1), child(8), child(9)];
        children[1] = vec![child(2), link(9, Edge::Blocks, false)];
        children[2] = vec![child(3), child(4)];
        children[4] = vec![child(5)];
        children[5] = vec![child(6)];
        children[6] = vec![child(7)];
        let total = beads.len();
        snapshot(
            vec![Tree {
                beads,
                children,
                counts: counts(0, total, 0, 0),
                ..tree(PROJECT, "smt-9lh.0", titles[0], counts(0, 0, 0, 0))
            }],
            Vec::new(),
            ProviderState::Absent,
        )
    }

    /// The elements a phone draws as rails on line `at`, each as its tag and
    /// its class: the levels inside what the line hangs under.
    fn rails(html: &str, at: usize) -> Vec<String> {
        let line = line_html(html, at);
        let hung = &line[..element_end(line, 0)];
        let content = hung.find('>').expect("the tag ends") + 1;
        parts(&hung[content..hung.rfind('<').expect("a closing tag")]).0
    }

    /// A phone draws a rail for each level a line hangs under, except the
    /// column a project's roots hang in: a trunk where a line below still
    /// hangs at that level, a gap where none does, and the line's own elbow,
    /// marked where it is the last there and where it hangs on a blocking
    /// edge. A line deeper than a phone indents keeps its outermost levels
    /// and its elbow.
    #[test]
    fn a_phone_draws_each_level_a_line_hangs_under_as_a_rail_up_to_five() {
        let html = page_of(deep(), "/");
        let drawn: Vec<(String, Vec<String>)> = (1..lines_html(&html).len())
            .map(|at| {
                (
                    words(&text_of(inside(line_html(&html, at), "title"))),
                    rails(&html, at),
                )
            })
            .collect();
        let rails = |title: &str, rails: &[&str]| {
            (
                title.to_string(),
                rails.iter().map(|rail| rail.to_string()).collect(),
            )
        };
        assert_eq!(
            drawn,
            [
                rails("refit the lighthouse", &[]),
                rails("rewire the lamp", &["u"]),
                rails("strip the old wiring", &["i", "u"]),
                rails("trace the feed", &["i", "i", "u"]),
                rails("open the junction box", &["i", "i", "u.last"]),
                rails("free the seized screw", &["i", "i", "b", "u.last"]),
                rails("soak it in oil", &["i", "i", "b", "b", "u.last"]),
                rails("wait a night", &["i", "i", "b", "b", "u.last"]),
                rails("log the burn", &["i", "u.last blocks"]),
                rails("test the fog signal", &["u"]),
                rails("log the burn", &["u.last"]),
            ]
        );
    }

    /// A list's rows hang under nothing, so a line waiting under one is
    /// drawn a rail from its first level.
    #[test]
    fn a_phone_draws_a_rail_from_the_first_level_under_a_lists_row() {
        let ready = page_of(
            deep(),
            &link_saying(&page_of(deep(), "/"), phrase::ready_only()),
        );
        let row = lines_titled(&ready, "wait a night")[0];
        let open = page_of(deep(), &fold_link_whole(&ready, row).0);
        let waiting = lines_titled(&open, "soak it in oil")[0];
        assert_eq!(rails(&open, row), Vec::<String>::new());
        assert_eq!(rails(&open, waiting), ["u.last up"]);
    }

    /// A phone draws a fold's arrow itself, so a fold says whether it is
    /// shut.
    #[test]
    fn a_shut_fold_says_so_and_an_open_one_does_not() {
        let html = page_at("/").html;
        let folds: Vec<(String, bool)> = (0..lines_html(&html).len())
            .filter(|&at| line_html(&html, at).contains("class=\"bead"))
            .filter(|&at| line_html(&html, at).starts_with("<a class=\"fold"))
            .map(|at| {
                let line = line_html(&html, at);
                (
                    words(&text_of(inside(line, "title"))),
                    line.starts_with("<a class=\"fold shut"),
                )
            })
            .collect();
        assert_eq!(
            folds,
            [
                ("lift the ground station".to_string(), false),
                ("wire the niri theme include".to_string(), false),
                ("apply the two niri settings".to_string(), true),
            ]
        );
    }

    /// A phone lets the bead view's prose flow rather than keep the rows the
    /// terminal's width broke it into. So each such break is an element of
    /// its own, and with every one of them taken out a paragraph reads whole,
    /// while an item the author started on a line of its own keeps its line.
    #[test]
    fn a_break_put_in_the_bead_views_prose_to_fit_the_terminal_can_be_taken_out() {
        let paragraph = "The diaphone has not sounded since the spring, so run it for a full cycle once the reflector is back in and record each burn in https://example.invalid/dunwich/logbook/diaphone/the-spring-cycle-once-the-reflector-is-back-in-and-every-burn-recorded.";
        let snapshot = with_the_bead_changed(|bead| {
            bead.row = crate::model::types::Printed::saying(
                &format!("{paragraph}\n\n- one item\n- another item"),
                "",
            );
        });
        let html = page_of(snapshot, &address(&key("smt-4kd3p.6")));
        let view = inside(&html, "page");
        assert!(view.contains("<span class=\"w\">\n"), "{view}");

        let mut flowed = String::new();
        let mut rest = view;
        while let Some(at) = rest.find("<span class=\"w\">") {
            flowed.push_str(&rest[..at]);
            rest = &rest[element_end(rest, at)..];
        }
        flowed.push_str(rest);
        let lines: Vec<String> = text_of(&flowed).lines().map(words).collect();
        assert!(lines.contains(&paragraph.to_string()), "{lines:#?}");
        assert!(lines.contains(&"• one item".to_string()), "{lines:#?}");
        assert!(lines.contains(&"• another item".to_string()), "{lines:#?}");
    }

    /// A phone sets the bead view's prose in a face of its own and lets it
    /// flow, and a wrapped item still hangs under its text rather than its
    /// marker. So each line of prose is an element holding its lead, the
    /// indent and any marker, apart from its text, and all of a wrapped
    /// item's rows are in the one element. A line of code is laid out by
    /// its author and is not prose.
    #[test]
    fn each_line_of_the_bead_views_prose_holds_its_lead_apart_from_its_text() {
        let item = "re-silver the reflector before the lamp goes back in, and test the diaphone once it is back in its housing on the gallery";
        let snapshot = with_the_bead_changed(|bead| {
            bead.row = crate::model::types::Printed::saying(
                &format!("A first paragraph.\n\n- {item}\n\n```\nlog the burn\n```"),
                "",
            );
        });
        let html = page_of(snapshot, &address(&key("smt-4kd3p.6")));
        let view = inside(&html, "page");
        let mut lines = Vec::new();
        let mut from = 0;
        while let Some(found) = view[from..].find("<span class=\"p\">") {
            let open = from + found;
            let end = element_end(view, open);
            let line = &view[open..end];
            let (shape, between) = parts(inside(line, "p"));
            assert_eq!(shape, ["span.lead", "span.t"], "{line}");
            assert_eq!(between, "", "{line}");
            let mut text = inside(line, "t").to_string();
            while let Some(at) = text.find("<span class=\"w\">") {
                text.replace_range(at..element_end(&text, at), "");
            }
            lines.push((text_of(inside(line, "lead")), words(&text_of(&text))));
            from = end;
        }
        assert!(
            lines.contains(&("  ".to_string(), "A first paragraph.".to_string())),
            "{lines:#?}"
        );
        assert!(
            lines.contains(&("  • ".to_string(), item.to_string())),
            "{lines:#?}"
        );
        assert!(
            !lines.iter().any(|(_, text)| text.contains("log the burn")),
            "{lines:#?}"
        );
        assert!(text_of(view).contains("log the burn"), "{view}");
    }

    /// A code span in the bead view's prose is marked as code, so a phone can
    /// set it in the terminal's face while the words round it are not.
    #[test]
    fn a_code_span_in_the_bead_views_prose_is_marked_as_code() {
        let snapshot = with_the_bead_changed(|bead| {
            bead.row = crate::model::types::Printed::saying("Trim the `wick` daily.", "");
        });
        let html = page_of(snapshot, &address(&key("smt-4kd3p.6")));
        let view = inside(&html, "page");
        let code: Vec<String> = view
            .match_indices("class=\"code\"")
            .map(|(at, _)| {
                let open = view[..at].rfind('<').expect("the attribute is in a tag");
                text_of(&view[open..element_end(view, open)])
            })
            .collect();
        assert_eq!(code, ["wick"], "{view}");
        assert!(STYLE.contains(".code{font-family:var(--code)"), "{STYLE}");
    }

    /// The page reads as a web page, in a proportional face and the page's
    /// own colours, while the tree's box-drawing keeps the code face its
    /// columns line up in.
    #[test]
    fn the_page_is_set_in_a_proportional_face_in_its_own_colours() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        assert!(html.contains(&palette::page_colours()), "{html}");
        assert!(
            STYLE.contains("body{margin:0;background:var(--surface);color:var(--fg);font:14px/1.45 var(--body)}"),
            "{STYLE}"
        );
        assert!(
            STYLE.contains(
                ".line>:has(+.bead),.line>.left>:not(:last-child),.code{font-family:var(--code)}"
            ),
            "{STYLE}"
        );
        assert!(
            STYLE.contains(".line .bead .code{font-size:inherit}"),
            "{STYLE}"
        );
        assert!(
            STYLE.contains(".bead .id{font:500 12px var(--body);color:var(--muted);"),
            "{STYLE}"
        );
    }

    /// The arrow a related bead is drawn after is apart from the reference,
    /// so a phone can draw the reference alone under its section's name.
    #[test]
    fn a_related_beads_arrow_is_apart_from_its_reference() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        let view = inside(&html, "page");
        let parent = view
            .find("<span class=\"arrow\">")
            .expect("the parent is drawn after its arrow");
        let (shape, _) = parts(&view[parent..element_end(view, parent)]);
        assert_eq!(shape, ["span.arrow"]);
        assert_eq!(words(&text_of(inside(&view[parent..], "arrow"))), "↑");
    }

    /// The references on a page, each whole, and the page with them taken
    /// out.
    fn references_and_the_rest(html: &str) -> (Vec<String>, String) {
        let mut references = Vec::new();
        let mut rest = String::new();
        let mut from = 0;
        while let Some(found) = html[from..].find("class=\"bead\"") {
            let open = html[..from + found].rfind('<').expect("an opening tag");
            rest.push_str(&html[from..open]);
            let end = element_end(html, open);
            references.push(html[open..end].to_string());
            from = end;
        }
        rest.push_str(&html[from..]);
        (references, rest)
    }

    /// Where the element opening at `open` ends, past its closing tag.
    fn element_end(html: &str, open: usize) -> usize {
        let mut depth = 0;
        let mut at = open;
        while let Some(tag) = html[at..].find('<') {
            let start = at + tag;
            let close = start + html[start..].find('>').expect("the tag ends") + 1;
            if html[start..close].starts_with("</") {
                depth -= 1;
            } else {
                depth += 1;
            }
            at = close;
            if depth == 0 {
                return at;
            }
        }
        html.len()
    }

    /// The address the link a page draws with the words `said` goes to, as a
    /// browser asks for it: without its fragment, which stays in the browser.
    fn link_saying(html: &str, said: &str) -> String {
        let end = [">{}</a>", ">{}<span class=\"count\">"]
            .iter()
            .find_map(|form| html.find(&form.replace("{}", &escaped(said))))
            .unwrap_or_else(|| panic!("a link says {said:?}: {html}"));
        let tag = &html[html[..end].rfind("<a ").expect("the link opens")..end];
        let href = tag
            .split("href=\"")
            .nth(1)
            .expect("the link goes somewhere");
        let href = href[..href.find('"').expect("the address ends")].replace("&amp;", "&");
        href.split('#').next().unwrap_or_default().to_string()
    }

    /// The terminal's forest for the ground after `keys`, each pressed on
    /// whatever the selection is on.
    fn pressed(keys: &[Action]) -> Forest {
        let mut forest = flatten(ground());
        for &key in keys {
            forest.apply(key);
        }
        forest
    }

    #[test]
    fn the_ready_switch_lists_what_the_terminal_lists_on_r_and_again_puts_the_trees_back() {
        let ready = page_at(&link_saying(&page_at("/").html, phrase::ready_only())).html;
        let terminal = terminal_lines(&pressed(&[Action::ToggleReadyList]));
        assert!(
            terminal
                .iter()
                .any(|line| line.contains("restore app theming")),
            "{terminal:#?}"
        );
        assert_eq!(page_lines(&ready), terminal);
        assert!(ready.contains("<a class=\"ready on\""), "{ready}");

        let back = page_at(&link_saying(&ready, phrase::ready_only())).html;
        assert_eq!(page_lines(&back), terminal_lines(&pressed(&[])));
    }

    /// Each view the config names is an entry of the menu, in the config's
    /// order, and picking it lists what `v` lists on reaching it. Picked
    /// with only ready beads listed, it keeps to them, as `v` keeps `r`.
    #[test]
    fn a_view_picked_from_the_menu_lists_what_the_terminal_lists_for_it() {
        let trees = page_at("/").html;
        let ready_trees = page_at(&link_saying(&trees, phrase::ready_only())).html;
        for (nth, (view, _)) in VIEWS.iter().enumerate() {
            let mut keys = vec![Action::NextView; nth + 1];
            let picked = page_at(&link_saying(&trees, view)).html;
            assert_eq!(
                page_lines(&picked),
                terminal_lines(&pressed(&keys)),
                "{view}"
            );

            let ready = page_at(&link_saying(&picked, phrase::ready_only())).html;
            keys.push(Action::ToggleReadyList);
            assert_eq!(
                page_lines(&ready),
                terminal_lines(&pressed(&keys)),
                "{view}"
            );
            let from_ready = page_at(&link_saying(&ready_trees, view)).html;
            assert_eq!(page_lines(&from_ready), page_lines(&ready), "{view}");
        }
        let left = page_at(&link_saying(
            &page_at(&link_saying(&trees, VIEWS[0].0)).html,
            phrase::every_tree(),
        ))
        .html;
        assert_eq!(page_lines(&left), terminal_lines(&pressed(&[])));
    }

    /// The beads the terminal's list counts in its one section.
    fn listed_count(forest: &Forest) -> usize {
        forest
            .lines()
            .iter()
            .find_map(|line| match line.content {
                Content::Section { count, .. } => Some(count),
                _ => None,
            })
            .expect("a view's list has a section")
    }

    /// The count of beads the menu gives beside `view`.
    fn count_beside(html: &str, view: &str) -> String {
        let at = html
            .find(&format!(">{}<span class=\"count\">", escaped(view)))
            .unwrap_or_else(|| panic!("the menu counts {view:?}: {html}"));
        let count = &html[at..];
        let count = &count[count.find("\"count\">").expect("a count") + 8..];
        count[..count.find('<').expect("the count ends")].to_string()
    }

    #[test]
    fn the_search_field_prompts_for_a_title_or_id_under_a_short_label() {
        let html = page_at("/").html;
        assert!(html.contains(&format!("placeholder=\"{}\"", phrase::find_prompt())));
        assert!(html.contains(&format!("<span>{}</span></label>", phrase::find())));
    }

    /// Beside each view the menu says how many beads picking it lists, and
    /// with only ready beads listed it counts only those, as picking it
    /// then keeps to them.
    #[test]
    fn the_menu_counts_the_beads_each_view_lists() {
        let trees = page_at("/").html;
        let ready_trees = page_at(&link_saying(&trees, phrase::ready_only())).html;
        for (nth, (view, _)) in VIEWS.iter().enumerate() {
            let mut keys = vec![Action::NextView; nth + 1];
            assert_eq!(
                count_beside(&trees, view),
                listed_count(&pressed(&keys)).to_string(),
                "{view}"
            );
            keys.push(Action::ToggleReadyList);
            assert_eq!(
                count_beside(&ready_trees, view),
                listed_count(&pressed(&keys)).to_string(),
                "{view} ready"
            );
        }
    }

    #[test]
    fn a_page_offers_no_list_menu_where_the_config_names_no_view() {
        let mut bare = ground();
        bare.views.clear();
        let asked = Asked::of("/").expect("the forest");
        let html = page(bare, &Layout::default(), &asked, drawing()).html;
        assert!(!html.contains("class=\"lists\""), "{html}");
        assert!(html.contains("class=\"ready\""), "{html}");
    }

    /// A bead's page offers its tree alone, as `F` on its row does, and the
    /// page it goes to draws what the terminal draws then, with the bead
    /// still shown. Its head says so and puts every tree back, as `F` again.
    #[test]
    fn a_bead_drawn_as_the_only_root_is_drawn_as_the_terminal_draws_it_on_f() {
        let bead = key("smt-4kd3p.6");
        let page_of_it = page_at(&address(&bead)).html;
        let rooted = page_at(&link_saying(&page_of_it, phrase::only_this_tree())).html;

        let mut terminal = flatten(ground());
        terminal.go_to(&bead);
        terminal.apply(Action::FocusForest);
        assert!(terminal.is_focused());
        let lines = terminal_lines(&terminal);
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("restore app theming")),
            "{lines:#?}"
        );
        assert_eq!(page_lines(&rooted), lines);
        assert_eq!(shown_line(&rooted), terminal.selected_line());
        assert!(
            text_of(&rooted).contains(phrase::showing_only()),
            "{rooted}"
        );
        assert!(!rooted.contains(phrase::only_this_tree()), "{rooted}");

        let whole = page_at(&link_saying(&rooted, phrase::show_all())).html;
        terminal.apply(Action::FocusForest);
        assert_eq!(page_lines(&whole), terminal_lines(&terminal));
        assert!(whole.contains(phrase::only_this_tree()), "{whole}");
    }

    /// The ready list keeps to the tree the forest is rooted at, as it does
    /// in the terminal.
    #[test]
    fn the_ready_list_keeps_to_the_bead_drawn_as_the_only_root() {
        let bead = key("smt-4kd3p.6");
        let rooted = page_at(&link_saying(
            &page_at(&address(&bead)).html,
            phrase::only_this_tree(),
        ))
        .html;
        let ready = page_at(&link_saying(&rooted, phrase::ready_only())).html;

        let mut terminal = flatten(ground());
        terminal.go_to(&bead);
        terminal.apply(Action::FocusForest);
        terminal.apply(Action::ToggleReadyList);
        assert_eq!(page_lines(&ready), terminal_lines(&terminal));
    }

    /// A search lands where the terminal's does, says what its foot says,
    /// and steps through the matches as `n` and `N` do.
    #[test]
    fn a_search_lands_and_steps_where_the_terminals_search_does() {
        let found = page_at("/?find=niri").html;
        let mut terminal = flatten(ground());
        let origin = terminal.origin();
        let landed = terminal.seek("niri", &origin);
        assert!(matches!(landed, Landed::On { of: 2, .. }), "{landed:?}");
        assert_eq!(page_lines(&found), terminal_lines(&terminal));
        assert_eq!(shown_line(&found), terminal.selected_line());
        assert!(
            text_of(&found).contains(&phrase::said(&Said::from(landed))),
            "{found}"
        );

        let next = page_at(&link_saying(&found, phrase::MATCH_AFTER.0)).html;
        let stepped = terminal.next_match(true).expect("a search to step through");
        assert_eq!(page_lines(&next), terminal_lines(&terminal));
        assert_eq!(shown_line(&next), terminal.selected_line());
        assert!(
            text_of(&next).contains(&phrase::said(&Said::from(stepped))),
            "{next}"
        );

        let back = page_at(&link_saying(&next, phrase::MATCH_BEFORE.0)).html;
        terminal.next_match(false);
        assert_eq!(page_lines(&back), terminal_lines(&terminal));
        assert_eq!(shown_line(&back), terminal.selected_line());
    }

    /// A step shuts what the step before it opened, as `n` and `N` do, so
    /// stepping back out of a deeper match shuts the fold opened for it.
    #[test]
    fn a_step_shuts_what_the_step_before_it_opened() {
        let found = page_at("/?find=wedge").html;
        let next = page_at(&link_saying(&found, phrase::MATCH_AFTER.0)).html;
        let back = page_at(&link_saying(&next, phrase::MATCH_BEFORE.0)).html;

        let mut terminal = flatten(ground());
        let origin = terminal.origin();
        terminal.seek("wedge", &origin);
        terminal.next_match(true);
        assert_eq!(page_lines(&next), terminal_lines(&terminal));
        terminal.next_match(false);
        let lines = terminal_lines(&terminal);
        assert!(
            !lines.iter().any(|line| line.contains("read the wedge log")),
            "{lines:#?}"
        );
        assert_eq!(page_lines(&back), lines);
        assert_eq!(shown_line(&back), terminal.selected_line());
    }

    /// A browser sends what is typed into the search with each space as a
    /// `+`, and the search is for the words with their spaces.
    #[test]
    fn a_search_typed_into_the_form_is_read_with_its_spaces() {
        let found = page_at("/?find=wedge+log").html;
        let mut terminal = flatten(ground());
        let origin = terminal.origin();
        let landed = terminal.seek("wedge log", &origin);
        assert!(matches!(landed, Landed::On { of: 1, .. }), "{landed:?}");
        assert_eq!(page_lines(&found), terminal_lines(&terminal));
        assert_eq!(shown_line(&found), terminal.selected_line());
        assert!(found.contains("value=\"wedge log\""), "{found}");
    }

    #[test]
    fn a_search_that_matches_nothing_says_so_and_leaves_the_forest_as_it_was() {
        let found = page_at("/?find=lighthouse").html;
        assert_eq!(page_lines(&found), terminal_lines(&pressed(&[])));
        assert!(!found.contains("line shown"), "{found}");
        let said = phrase::said(&Said::NothingMatched("lighthouse".to_string()));
        assert!(text_of(&found).contains(&said), "{found}");
    }

    /// A search looks through the list drawn, as the terminal's does, and
    /// the form carries the list on to the page it answers with.
    #[test]
    fn a_search_from_the_ready_list_looks_through_the_ready_list() {
        let ready = page_at(&link_saying(&page_at("/").html, phrase::ready_only())).html;
        assert!(
            ready.contains("<input type=\"hidden\" name=\"ready\" value=\"1\">"),
            "{ready}"
        );
        let found = page_at("/?ready=1&find=theming").html;
        let mut terminal = pressed(&[Action::ToggleReadyList]);
        let origin = terminal.origin();
        terminal.seek("theming", &origin);
        assert_eq!(page_lines(&found), terminal_lines(&terminal));
        assert_eq!(shown_line(&found), terminal.selected_line());
    }

    /// What each reference to a bead on `html` asks for, by the bead's id,
    /// after checking the page it opens shows it.
    fn references_on(html: &str) -> HashMap<String, Asked> {
        html.split("<a class=\"bead\" href=\"")
            .skip(1)
            .map(|link| {
                let link = link[..link.find('"').expect("the address ends")].replace("&amp;", "&");
                let target = link.split('#').next().unwrap_or_default();
                assert!(page_at(target).found, "{link} shows no bead");
                let asked = Asked::of(target).expect("an address");
                (asked.bead.clone().expect("a bead's address").id, asked)
            })
            .collect()
    }

    /// The same of the references the bead view on `html` draws.
    fn named_by_the_bead_view(html: &str) -> HashMap<String, Asked> {
        references_on(&html[html.find("<aside>").expect("the bead view")..])
    }

    /// Where a list has a row expanded to the beads waiting on it, a bead
    /// that waits and has no row of its own opens with every tree drawn.
    #[test]
    fn a_bead_waiting_under_a_listed_row_opens_a_page_that_shows_it() {
        let ready = page_at("/?ready=1").html;
        let row = page_lines(&ready)
            .iter()
            .position(|line| line.contains("restore app theming"))
            .expect("the ready bead is listed");
        let expanded = page_at(&fold_link(&ready, row)).html;
        assert!(
            page_lines(&expanded)
                .iter()
                .any(|line| line.contains("wire the niri theme include")),
            "{expanded}"
        );
        let references = references_on(&expanded);
        assert!(references["smt-4kd3p.1"].list.is_none(), "{expanded}");
        assert!(references["smt-4kd3p.4"].list.is_some(), "{expanded}");
    }

    /// The band over a page drawn with one bead as its only root says the
    /// page shows only that bead, names it by its title, then offers every
    /// tree back.
    #[test]
    fn the_band_over_one_tree_says_showing_only_the_bead_then_show_all_trees() {
        let bead = key("smt-4kd3p.6");
        let rooted = page_at(&link_saying(
            &page_at(&address(&bead)).html,
            phrase::only_this_tree(),
        ))
        .html;
        let band = &rooted[rooted.find("<p class=\"rooted\">").expect("the band")..];
        let band = text_of(&band[..band.find("</p>").expect("the band ends")]);
        let ground = ground();
        let title = &ground.node(&bead).expect("the bead is held").title;
        let said = [phrase::showing_only(), title, phrase::show_all()].map(|said| {
            band.find(said)
                .unwrap_or_else(|| panic!("{said:?} in {band:?}"))
        });
        assert!(said.is_sorted(), "{band:?}");
        assert_eq!(phrase::show_all(), "Show all trees");
    }

    /// The bead the forest is drawn with as its only root opens from the
    /// head of the page even where the list drawn leaves it out.
    #[test]
    fn the_only_root_opens_from_the_head_where_the_list_leaves_it_out() {
        let bead = key("smt-4kd3p.6");
        let rooted = page_at(&link_saying(
            &page_at(&address(&bead)).html,
            phrase::only_this_tree(),
        ))
        .html;
        let ready = page_at(&link_saying(&rooted, phrase::ready_only())).html;
        let head = &ready[ready.find("<p class=\"rooted\">").expect("the head")..];
        let head = &head[..head.find("</p>").expect("the head ends")];
        assert!(references_on(head).contains_key(&bead.id), "{head}");
    }

    /// What a browser asks for when `typed` is searched for with the form on
    /// `html`: the form's hidden pairs, then what was typed.
    fn searched_from(html: &str, typed: &str) -> String {
        let form = &html[html.find("<form").expect("a search form")..];
        let form = &form[..form.find("</form>").expect("the form ends")];
        let written = |value: &str| {
            value
                .replace("&amp;", "&")
                .replace('%', "%25")
                .replace('&', "%26")
                .replace('+', "%2B")
                .replace(' ', "+")
        };
        let pairs: Vec<String> = form
            .split("<input type=\"hidden\" name=\"")
            .skip(1)
            .map(|input| {
                let (name, rest) = input.split_once("\" value=\"").expect("a value");
                format!(
                    "{name}={}",
                    written(&rest[..rest.find('"').expect("it ends")])
                )
            })
            .chain([format!("find={}", written(typed))])
            .collect();
        format!("/?{}", pairs.join("&"))
    }

    /// A search from a bead's page counts from that bead, as the terminal's
    /// counts from the selection.
    #[test]
    fn a_search_from_a_beads_page_counts_from_that_bead() {
        let bead = key("smt-4kd3p.4");
        let found = page_at(&searched_from(&page_at(&address(&bead)).html, "niri")).html;

        let mut terminal = flatten(ground());
        terminal.go_to(&bead);
        let origin = terminal.origin();
        let landed = terminal.seek("niri", &origin);
        assert!(
            matches!(&landed, Landed::On { key, .. } if key.id == "smt-4kd3p.6"),
            "{landed:?}"
        );
        assert_eq!(page_lines(&found), terminal_lines(&terminal));
        assert_eq!(shown_line(&found), terminal.selected_line());
    }

    /// A fold the reader pointed under a line they shut is still pointed
    /// once a search opens the line.
    #[test]
    fn a_search_that_opens_a_shut_line_keeps_the_folds_pointed_under_it() {
        let lines = terminal_lines(&terminal_forest(&[]));
        let under = lines
            .iter()
            .position(|line| line.contains("▸ ○ .6"))
            .expect("the blocked bead rests shut over its child");
        let opened = page_at(&fold_link(&page_at("/").html, under)).html;
        let nested = page_lines(&opened)
            .iter()
            .position(|line| line.contains("confirm the wedges are gone"))
            .expect("its child is drawn");
        let nested_open = page_at(&fold_link(&opened, nested)).html;
        let shut = page_at(&fold_link(&nested_open, under)).html;
        let found = page_at(&searched_from(&shut, "confirm")).html;

        let mut terminal = terminal_forest(&[under, nested, under]);
        let origin = terminal.origin();
        terminal.seek("confirm", &origin);
        let lines = terminal_lines(&terminal);
        assert!(
            lines.iter().any(|line| line.contains("read the wedge log")),
            "{lines:#?}"
        );
        assert_eq!(page_lines(&found), lines);
        assert_eq!(shown_line(&found), terminal.selected_line());
    }

    /// A fold opened in the trees leaves the same bead's row in a list as
    /// it was, as the terminal keeps what a list's row unblocks apart from
    /// the trees' folds.
    #[test]
    fn a_fold_opened_in_the_trees_leaves_a_lists_row_of_the_bead_shut() {
        let shut = terminal_lines(&terminal_forest(&[]))
            .iter()
            .position(|line| line.contains("▸ ○ .6"))
            .expect("the blocked bead rests shut over its child");
        let opened = page_at(&fold_link(&page_at("/").html, shut)).html;
        let view = page_at(&link_saying(&opened, VIEWS[1].0)).html;

        let mut terminal = terminal_forest(&[shut]);
        terminal.apply(Action::NextView);
        terminal.apply(Action::NextView);
        assert_eq!(page_lines(&view), terminal_lines(&terminal));
    }

    /// Stepping through matches under a line the reader shut keeps the folds
    /// they pointed beneath it, as the terminal's steps do.
    #[test]
    fn stepping_under_a_shut_line_keeps_the_folds_pointed_beneath_it() {
        let under = terminal_lines(&terminal_forest(&[]))
            .iter()
            .position(|line| line.contains("▸ ○ .6"))
            .expect("the blocked bead rests shut over its child");
        let opened = page_at(&fold_link(&page_at("/").html, under)).html;
        let nested = page_lines(&opened)
            .iter()
            .position(|line| line.contains("confirm the wedges are gone"))
            .expect("its child is drawn");
        let nested_open = page_at(&fold_link(&opened, nested)).html;
        let shut = page_at(&fold_link(&nested_open, under)).html;
        let found = page_at(&searched_from(&shut, "wedge")).html;
        let next = page_at(&link_saying(&found, phrase::MATCH_AFTER.0)).html;
        let back = page_at(&link_saying(&next, phrase::MATCH_BEFORE.0)).html;

        let mut terminal = terminal_forest(&[under, nested, under]);
        let origin = terminal.origin();
        terminal.seek("wedge", &origin);
        assert_eq!(page_lines(&found), terminal_lines(&terminal));
        terminal.next_match(true);
        assert_eq!(page_lines(&next), terminal_lines(&terminal));
        terminal.next_match(false);
        assert_eq!(page_lines(&back), terminal_lines(&terminal));
    }

    /// A bead's address in a view, naming no copy, shows the copy the view
    /// lists, where the tree reaches the bead twice.
    #[test]
    fn a_beads_address_in_a_view_shows_the_copy_the_view_lists() {
        let target = format!("{}?view={}", address(&key("smt-4kd3p.6")), VIEWS[0].0);
        let asked = Asked::of(&target).expect("an address");
        assert!(page(reached_twice(), &Layout::default(), &asked, drawing()).found);
    }

    /// A list's row expanded to what it unblocks stays expanded when the
    /// forest is drawn with one bead as its only root, as the terminal keeps
    /// what each bead's row unblocks by the bead.
    #[test]
    fn a_lists_row_expanded_is_known_by_its_bead() {
        let ready = page_at("/?ready=1").html;
        let row = page_lines(&ready)
            .iter()
            .position(|line| line.contains("restore app theming"))
            .expect("the ready bead is listed");
        let link = fold_link(&ready, row);
        let asked = Asked::of(&link).expect("an address");
        let name = &asked.folds.last().expect("the row's fold").name;
        assert_eq!(name, "u:summit-works:smt-4kd3p.4");
    }

    /// A list's rows are peers, with no folds over one another, so a bead's
    /// page in a view opens no other row on the way to it, even one that
    /// is its parent in the tree.
    #[test]
    fn a_beads_page_in_a_view_opens_no_other_row_to_reach_it() {
        let target = format!("{}?view={}", address(&key("smt-4kd3p.7")), VIEWS[1].0);
        let answer = page_at(&target);
        assert!(answer.found, "{}", answer.html);
        let closing = link_saying(&answer.html, phrase::back());
        let carried = Asked::of(&closing).expect("an address").folds;
        assert_eq!(carried, Vec::new(), "{closing}");
    }

    /// A search from a page that marks no selection counts from the top,
    /// wherever pointing the page's folds left the selection.
    #[test]
    fn a_search_from_a_page_with_no_selection_counts_from_the_top() {
        let under = terminal_lines(&terminal_forest(&[]))
            .iter()
            .position(|line| line.contains("▸ ○ .6"))
            .expect("the blocked bead rests shut over its child");
        let opened = page_at(&fold_link(&page_at("/").html, under)).html;
        let nested = page_lines(&opened)
            .iter()
            .position(|line| line.contains("confirm the wedges are gone"))
            .expect("its child is drawn");
        let nested_open = page_at(&fold_link(&opened, nested)).html;
        let found = page_at(&searched_from(&nested_open, "wedge")).html;

        let mut terminal = terminal_forest(&[under, nested]);
        terminal.select_line(0);
        let origin = terminal.origin();
        let landed = terminal.seek("wedge", &origin);
        assert!(
            matches!(&landed, Landed::On { key, .. } if key.id == "smt-4kd3p.7"),
            "{landed:?}"
        );
        assert_eq!(shown_line(&found), terminal.selected_line());
    }

    /// A key the page does not know is passed over whatever its value holds.
    #[test]
    fn an_address_with_a_key_the_page_does_not_know_is_still_read() {
        assert!(Asked::of("/?utm=%FF").is_some());
    }

    /// An address rooted at a bead the forest no longer draws says so over
    /// every tree, and offers them without the root.
    #[test]
    fn a_root_the_forest_no_longer_draws_is_said_to_be_gone() {
        let gone = key("smt-4kd3p.99");
        let html = page_at("/?root=summit-works%3Asmt-4kd3p%2Fsummit-works%3Asmt-4kd3p.99").html;
        assert!(text_of(&html).contains(&phrase::not_drawn(&gone)), "{html}");
        let all = link_saying(&html, phrase::show_all());
        assert_eq!(Asked::of(&all).and_then(|asked| asked.root), None, "{all}");
    }

    /// A search's form goes to the match, as its steps do.
    #[test]
    fn a_search_from_the_form_lands_the_page_on_the_match() {
        let html = page_at("/").html;
        assert!(
            html.contains(&format!("<form action=\"/#{SHOWN}\" method=\"get\">")),
            "{html}"
        );
    }

    /// A search that finds nothing leaves the selection where it began, as
    /// the terminal's does, so the next search counts from there too.
    #[test]
    fn a_search_after_one_that_found_nothing_counts_from_where_both_began() {
        let bead = key("smt-4kd3p.4");
        let nothing = page_at(&searched_from(&page_at(&address(&bead)).html, "lighthouse")).html;
        let found = page_at(&searched_from(&nothing, "niri")).html;

        let mut terminal = flatten(ground());
        terminal.go_to(&bead);
        let origin = terminal.origin();
        terminal.seek("niri", &origin);
        assert_eq!(shown_line(&found), terminal.selected_line());
    }

    /// Where the tree reaches a bead twice, a view lists one copy of it,
    /// and a reference to the bead from inside the view opens that copy in
    /// the view.
    #[test]
    fn a_bead_a_view_lists_opens_in_the_view_whichever_copy_it_lists() {
        let target = format!("{}?view={}", address(&key("smt-4kd3p.1")), VIEWS[0].0);
        let html = page_of(reached_twice(), &target);
        let aside = &html[html.find("<aside>").expect("the bead view")..];
        let link = aside
            .split("<a class=\"bead\" href=\"")
            .skip(1)
            .map(|link| link[..link.find('"').expect("the address ends")].replace("&amp;", "&"))
            .find(|link| link.starts_with(&address(&key("smt-4kd3p.6"))))
            .expect("the bead view names the bead it blocks");
        let target = link.split('#').next().unwrap_or_default();
        assert!(
            Asked::of(target).and_then(|asked| asked.list).is_some(),
            "{link}"
        );
        let asked = Asked::of(target).expect("an address");
        assert!(
            page(reached_twice(), &Layout::default(), &asked, drawing()).found,
            "{link}"
        );
    }

    /// A fold pointed on a search's answer is pointed on the forest the
    /// search opened, as a fold key after `/` is: the folds the search
    /// opened stay open.
    #[test]
    fn a_fold_pointed_on_a_searchs_answer_keeps_what_the_search_opened() {
        let found = page_at("/?find=wedge").html;
        let on = shown_line(&found);
        let toggled = page_at(&fold_link(&found, on)).html;

        let mut terminal = flatten(ground());
        let origin = terminal.origin();
        terminal.seek("wedge", &origin);
        terminal.select_line(on);
        terminal.apply(Action::ToggleFold);
        let lines = terminal_lines(&terminal);
        assert!(
            lines.iter().any(|line| line.contains("read the wedge log")),
            "{lines:#?}"
        );
        assert_eq!(page_lines(&toggled), lines);
    }

    /// Every bead the bead view names opens a page that shows it. One
    /// outside the tree the forest is drawn with as its only root opens with
    /// every tree drawn, and one inside it keeps the root.
    #[test]
    fn a_bead_named_from_inside_the_only_root_opens_a_page_that_shows_it() {
        let rooted = page_at(&link_saying(
            &page_at(&address(&key("smt-4kd3p.6"))).html,
            phrase::only_this_tree(),
        ))
        .html;
        let named = named_by_the_bead_view(&rooted);
        assert!(
            named["smt-4kd3p"].root.is_none(),
            "the parent is outside the tree"
        );
        assert!(
            named["smt-4kd3p.6"].root.is_some(),
            "the root itself is inside it"
        );
    }

    /// The same of a view: a bead it does not list opens with every tree
    /// drawn, and one it lists keeps the view.
    #[test]
    fn a_bead_named_from_inside_a_view_opens_a_page_that_shows_it() {
        let view = format!("{}?view={}", address(&key("smt-4kd3p.6")), VIEWS[0].0);
        let named = named_by_the_bead_view(&page_at(&view).html);
        assert!(
            named["smt-4kd3p"].list.is_none(),
            "the view leaves the parent out"
        );
        assert!(
            named["smt-4kd3p.6"].list.is_some(),
            "the view lists the bead"
        );
    }

    /// A config may name a view with nothing, and its menu entry still
    /// picks it.
    #[test]
    fn a_view_named_with_nothing_is_picked_by_its_address() {
        let asked = Asked::default().listing(listed(Some(String::new()), false));
        assert_eq!(
            Asked::of(&asked).expect("an address").list,
            listed(Some(String::new()), false)
        );
    }

    /// Every link a page carries keeps the list it draws and the bead it is
    /// rooted at, as every key leaves them in the terminal.
    #[test]
    fn every_link_keeps_the_list_and_the_root() {
        let place = |id: &str| {
            let mut forest = flatten(ground());
            forest.go_to(&key(id));
            forest.place().cloned().expect("the bead is drawn")
        };
        let asked = Asked {
            list: Some(List::View {
                name: VIEWS[1].0.to_string(),
                ready: true,
            }),
            root: Some(place("smt-4kd3p.6")),
            ..Asked::default()
        };
        for link in [
            asked.opening(&key("smt-4kd3p.7")),
            asked.opening_at(&place("smt-4kd3p.7")),
            asked.closing(None, false),
            asked.listing(asked.list.clone()),
            asked.rooting(asked.root.as_ref()),
        ] {
            let read = Asked::of(link.split('#').next().unwrap_or_default())
                .expect("the link is an address");
            assert_eq!(
                (&read.list, &read.root),
                (&asked.list, &asked.root),
                "{link}"
            );
        }
    }
}
