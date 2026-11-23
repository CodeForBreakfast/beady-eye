//! The forest and the bead drawn as a web page rather than into a frame.
//!
//! Every line is the line the terminal draws, in its words and its colours,
//! written as HTML. What the terminal does with a key the page does with a
//! link, and every link's address is `view::address`'s to compose: a line
//! that folds links to the page with that fold toggled, and every reference
//! to a bead, a row of the forest or a bead the bead view names, is one
//! component that opens the bead.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, FixedOffset, Utc};
use ratatui::text::Span;

use crate::app::Awaited;
use crate::model::edges::Related;
use crate::model::join::BeadKey;
use crate::model::snapshot::{Node, Snapshot};
use crate::model::types::{Edge, Status};
use crate::view::address::{line_name, Asked, FIND, SHOWN};
use crate::view::fitted::{indent, Block, Mark, Marked};
use crate::view::forest::{listed, only_ready, view_of, Forest, Landed, List};
use crate::view::lines::{self, Content, Level, Line, Note, ProjectLine, Waiting};
use crate::view::markdown::Flow;
use crate::view::row::{Cell, Layout, Progress, WARNING};
use crate::view::{palette, phrase, show, Said};

use super::bead::{elided_run, referenced_line};
use super::foot::notices;
use super::project::wants_attention;
use super::{done, fitted, Reads, Reference};

/// The columns the bead view wraps its prose at, which is what `bd show`
/// wraps at.
const PROSE: usize = 80;

/// The page a request asked for, and whether it holds what was asked for: a
/// bead's address names a bead the forest draws, or it does not.
pub struct Answer {
    pub found: bool,
    pub html: String,
}

/// When a page is drawn, the reads out as it is drawn, and the change to
/// what the server holds that it is drawn from, which its script asks for
/// the changes since. `offset` is the zone a browser that runs no script
/// sees a time in full in.
#[derive(Debug, Clone)]
pub struct Drawn {
    pub at: DateTime<Utc>,
    pub offset: FixedOffset,
    pub collecting: Vec<Awaited>,
    pub change: u64,
}

/// The page `asked` names, drawn from `snapshot` under `layout`.
///
/// Each project's line says the time of day it was read at, which stays
/// the same however often the page is drawn again, so a page kept current
/// is sent no change for it. The page's script says how long ago instead,
/// as the terminal does, and keeps that true on a page left open.
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
    if shown.bead.is_none() {
        drawing.title(&mut html, drawn.at, drawn.offset);
    }
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
            None => html.push_str(&format!("<p class=\"missing\">{}</p>", not_in_any_tree(key))),
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

/// The warning for a bead an address names that no tree the forest draws
/// holds, with the bead's id set quietly after it.
fn not_in_any_tree(key: &BeadKey) -> String {
    format!(
        "{} <span class=\"id\">{}</span>",
        escaped(&format!("{WARNING} {}", phrase::not_in_any_tree())),
        escaped(&key.id)
    )
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
            format!("<script>{IN_READERS_ZONE}{KEPT_CURRENT}{ESCAPE_CLOSES}</script>"),
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
    /// The name of the list the page draws, which heads it on a phone, and
    /// beside it when the oldest read of the projects it draws was, the one
    /// claim true of every row.
    fn title(&self, html: &mut String, now: DateTime<Utc>, offset: FixedOffset) {
        let list = self.forest.list();
        let named = if list == Some(&List::Ready) {
            phrase::ready_only().to_string()
        } else {
            menu_name(list)
        };
        html.push_str(&format!(
            "<header class=\"title\"><h1>{}</h1>",
            escaped(&named)
        ));
        let lines = self.forest.lines();
        let read_at = &self.forest.snapshot().read_at;
        let reads: Option<Vec<_>> = lines
            .viewport(0, lines.len())
            .filter_map(|(_, line)| match &line.content {
                Content::Project(project) => Some(read_at.get(&project.project)),
                _ => None,
            })
            .collect();
        if let Some(&oldest) = reads.and_then(|reads| reads.into_iter().min()) {
            read_time(html, oldest, offset, now);
        }
        html.push_str("</header>\n");
    }

    /// What the terminal's keys put in force over the whole page, as links
    /// and a form: the list menu, the ready switch and the search, as `v`,
    /// `r` and `/`. The menu offers the trees, the beads to check and each
    /// view the config names, each list with how many beads it holds. Where
    /// there are beads to check, a button beside the switch lists them too.
    /// Then, where the forest is drawn with one bead as its only root, that
    /// bead and the link that puts every tree back, as `F` does.
    fn nav(&self, html: &mut String) {
        let list = self.forest.list();
        let ready = only_ready(list);
        let drawn = view_of(list);
        let counts = self.forest.list_counts(ready);
        let to_check = counts.to_check;
        let every_check = Some(List::ToCheck { project: None });
        let checking_every_project = list == every_check.as_ref();
        let checked = self.asked.listing(every_check);
        let named = menu_name(list);
        html.push_str(&format!(
            "<nav><span class=\"brand\">{}</span>\
             <details class=\"lists\"><summary><span>{}</span></summary><div>",
            phrase::PRODUCT,
            escaped(&named)
        ));
        html.push_str(&format!(
            "<a href=\"{}\"{}>{}</a>\
             <a href=\"{}\"{}>{}<span class=\"count\">{to_check}</span></a>",
            escaped(&self.asked.listing(listed(None, ready))),
            on(matches!(list, None | Some(List::Ready))),
            escaped(phrase::every_tree()),
            escaped(&checked),
            on(checking_every_project),
            escaped(phrase::to_check())
        ));
        let views = &self.forest.snapshot().views;
        if !views.is_empty() {
            html.push_str(&format!("<p class=\"hd\">{}</p>", escaped(phrase::views())));
            for (view, count) in views.iter().zip(counts.views) {
                html.push_str(&format!(
                    "<a href=\"{}\"{}>{}<span class=\"count\">{count}</span></a>",
                    escaped(&self.asked.listing(listed(Some(view.clone()), ready))),
                    on(Some(view.as_str()) == drawn),
                    escaped(view)
                ));
            }
        }
        html.push_str("</div></details>");
        html.push_str(&format!(
            "<a class=\"ready{}\" href=\"{}\">{}</a>",
            if ready { " on" } else { "" },
            escaped(&self.asked.toggling_ready()),
            escaped(phrase::ready_only())
        ));
        if to_check > 0 {
            html.push_str(&format!(
                "<a class=\"check{}\" href=\"{}\">{}</a>",
                if checking_every_project { " on" } else { "" },
                escaped(&checked),
                escaped(&format!(
                    "{WARNING} {}",
                    phrase::anomalies_beneath(to_check)
                ))
            ));
        }
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
        match self
            .forest
            .snapshot()
            .node(root.key())
            .filter(|_| self.forest.is_focused())
        {
            // The list drawn can leave the root out, so the band links it.
            Some(node) => {
                html.push_str(&format!(
                    "<p class=\"rooted\"><span class=\"said\">{}</span> ",
                    escaped(phrase::showing_only())
                ));
                self.reference(
                    html,
                    show::named(node, vec![Span::raw(phrase::this_tree())]),
                    Some(self.opening(root.key())),
                    None,
                );
            }
            None => html.push_str(&format!(
                "<p class=\"missing\">{}",
                not_in_any_tree(root.key())
            )),
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
        // A project's heading and the lines under it are one section, so the
        // heading sticks only while its own lines scroll.
        let mut in_project = false;
        let mut folding = false;
        for (at, line) in lines.viewport(0, lines.len()) {
            if in_project && line.depth == 0 {
                html.push_str("</section>");
                in_project = false;
            }
            let heading = matches!(line.content, Content::Project(_));
            if folding && line.depth == 0 && !heading {
                html.push_str("</details>");
                folding = false;
            }
            match &line.content {
                // Counted in the top bar and the menu, which list those beads.
                Content::Note(Note::ToCheck(_)) => {}
                Content::Project(project) => {
                    if !folding && self.across_projects() {
                        self.folded_line(html, &reads, drawn.offset);
                        folding = true;
                    }
                    html.push_str("<section>");
                    in_project = true;
                    self.open_line(html, at, line, " heading", String::new());
                    self.heading(html, line, project, &reads, drawn.offset);
                }
                _ => self.forest_line(html, at, line, &reads),
            }
        }
        if in_project {
            html.push_str("</section>");
        }
        if folding {
            html.push_str("</details>");
        }
    }

    /// The one line a list across projects folds their headings into, which
    /// opens to them: how many projects have no read gone badly and, once
    /// every one of those has been read, when the oldest of them was, then
    /// each project whose read went badly, named in the colour of a flag
    /// beside the mark its heading draws. A project whose first read is still
    /// out is counted and holds the time back, as it does in the title.
    fn folded_line(&self, html: &mut String, reads: &Reads, offset: FixedOffset) {
        let lines = self.forest.lines();
        let (troubled, fresh): (Vec<_>, Vec<_>) = lines
            .viewport(0, lines.len())
            .filter_map(|(_, line)| match &line.content {
                Content::Project(project) => Some((project, reads.of(project))),
                _ => None,
            })
            .partition(|(_, how_fresh)| how_fresh.is_some_and(|fresh| wants_attention(fresh.mark)));
        html.push_str("<details class=\"projects\"><summary>");
        if !fresh.is_empty() {
            html.push_str(&format!(
                "<span class=\"fresh\">{}",
                escaped(&phrase::projects(fresh.len()))
            ));
            let read: Option<Vec<_>> = fresh
                .iter()
                .map(|(_, how_fresh)| how_fresh.and_then(|fresh| fresh.read_at))
                .collect();
            if let Some(oldest) = read.and_then(|read| read.into_iter().min()) {
                read_time(html, oldest, offset, reads.now);
            }
            html.push_str("</span>");
        }
        let troubled = troubled
            .into_iter()
            .filter_map(|(project, how_fresh)| Some((project, how_fresh?)));
        for (project, how_fresh) in troubled {
            html.push_str(&format!(
                "<span class=\"flag\">{} {}</span>",
                escaped(phrase::mark(how_fresh, reads.now)),
                escaped(&project.project)
            ));
        }
        html.push_str("</summary>");
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
    /// check. The terminal says the same on the project's line. What is left
    /// and what is to check each link to a list of those beads.
    fn heading(
        &self,
        html: &mut String,
        line: &Line,
        project: &ProjectLine,
        reads: &Reads,
        offset: FixedOffset,
    ) {
        html.push_str("<span class=\"left\">");
        self.prefix(
            html,
            line,
            &Span::raw(line.prefix.clone()),
            &project.project,
        );
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
            if let Some(at) = how_fresh.read_at {
                read_time(html, at, offset, reads.now);
            }
            html.push_str("</span>");
        }
        let counts = &project.counts;
        html.push_str("<span class=\"state\">");
        // Nothing counted means no root here read at all, which a bar would
        // say the opposite of.
        if counts.total > 0 {
            let left = Some(List::LeftToDo {
                project: project.project.clone(),
            });
            progress(
                html,
                counts.finished,
                counts.total,
                &phrase::left_of(counts.unfinished(), counts.total),
                Some(&self.asked.listing(left)),
            );
        }
        if counts.live_agents > 0 {
            html.push_str(&format!(
                "<span class=\"agents\">{}</span>",
                escaped(&phrase::agents(counts.live_agents))
            ));
        }
        if counts.anomalies > 0 {
            let to_check = Some(List::ToCheck {
                project: Some(project.project.clone()),
            });
            html.push_str(&format!(
                "<a class=\"flag\" href=\"{}\">{}</a>",
                escaped(&self.asked.listing(to_check)),
                escaped(&format!(
                    "{WARNING} {}",
                    phrase::anomalies_beneath(counts.anomalies)
                ))
            ));
        }
        html.push_str("</span></div>\n");
    }

    /// One line of the forest. A bead's row is the terminal's row with its
    /// glyph, id and title drawn as a reference to the bead, its agent and
    /// anomalies said in the page's own words, and its progress drawn last,
    /// as a bar in a column of its own. Where the layout draws progress,
    /// every bead's row has that column, empty where the bead has none, so
    /// the bars and what is drawn before them line up down the page.
    fn forest_line(&self, html: &mut String, at: usize, line: &Line, reads: &Reads) {
        let widths = self.forest.lines().widths();
        let (reference, progressed, drawn) = match &line.content {
            Content::Bead(row) | Content::Waiting(Waiting { row, .. }) => {
                let (reference, progressed, drawn) =
                    referenced_line(row, &line.prefix, widths, self.layout);
                (Some(reference), progressed, drawn)
            }
            Content::Elided { count, .. } => (
                None,
                None,
                elided_run(&line.prefix, &phrase::finished_run(*count)),
            ),
            _ => (None, None, fitted(line, widths, self.layout, reads)),
        };
        let column = reference.is_some() && self.layout.cells().any(|cell| *cell == Cell::Progress);
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
            let what = match &line.content {
                Content::Bead(row) | Content::Waiting(Waiting { row, .. }) => row.title.clone(),
                _ => identity
                    .iter()
                    .chain(whole.title)
                    .flat_map(|span| span.content.split_whitespace())
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            self.prefix(html, line, prefix, &what);
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
            self.reference(html, reference, href, self.named_project(line));
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
        if column && !whole.state.is_empty() {
            html.push_str(&indent());
        }
        match progressed {
            Some(Progress { finished, total }) => {
                progress(html, finished, total, &done(finished, total), None);
            }
            None if column => html.push_str("<span class=\"prog\"></span>"),
            None => {}
        }
        html.push_str("</span></div>\n");
    }

    /// Whether the lines are a list's rows, drawn in place of the trees.
    fn listed(&self) -> bool {
        self.forest.list().is_some()
    }

    /// The project a list's own row names, where the list draws beads of
    /// more than one project and no heading sits over each project's rows.
    /// A row waiting under a list's row is not the list's own.
    fn named_project<'l>(&self, line: &'l Line) -> Option<&'l str> {
        match (&line.content, &line.place) {
            (Content::Bead(_), Some(place)) if self.across_projects() => Some(&place.key().project),
            _ => None,
        }
    }

    /// Whether the lines are a list that draws beads of more than one
    /// project, with no heading over each project's rows.
    fn across_projects(&self) -> bool {
        self.forest
            .list()
            .is_some_and(|list| list.project().is_none())
            && self.forest.snapshot().projects.len() > 1
    }

    /// What a line hangs under, drawn as rails, then the control that
    /// toggles the line's fold where it has one, named for `what` it folds.
    fn prefix(&self, html: &mut String, line: &Line, prefix: &Span<'static>, what: &str) {
        let (drawn, class) = match railed(&prefix.content, self.listed()) {
            Some(drawn) => (drawn, " class=\"rails\""),
            None => (escaped(&prefix.content), ""),
        };
        html.push_str(&format!(
            "<span{class}{}>{drawn}",
            styled(palette::css(prefix.style)),
        ));
        if let Some(href) = self.asked.toggling(line, self.listed()) {
            let open = line.folded == Some(true);
            let (said, shut) = if open {
                (phrase::fold(what), "")
            } else {
                (phrase::unfold(what), " shut")
            };
            html.push_str(&format!(
                "<a class=\"fold{shut}\" href=\"{}\" aria-expanded=\"{open}\" aria-label=\"{}\"></a>",
                escaped(&href),
                escaped(&said),
            ));
        }
        html.push_str("</span>");
    }

    /// The one component every reference to a bead is drawn by: its status
    /// mark, its title, then its id set quietly, and a link to `href` where
    /// the page can open the bead. The mark is labelled with `bd`'s word for
    /// the status. The id takes no colour from the place drawing it, because
    /// a status colour or a finished row's tone would make it fainter still.
    /// Where the reference names its `project`, a quiet tag after the id
    /// says it.
    fn reference(
        &self,
        html: &mut String,
        reference: Reference,
        href: Option<String>,
        project: Option<&str>,
    ) {
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
        mark(html, &reference.status, &reference.glyph(), true);
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
        if let Some(project) = project {
            html.push_str(&format!(
                "<span class=\"project\">{}</span>",
                escaped(project)
            ));
        }
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
        self.head(html, node, page.blocked_by.is_some());
        html.push_str("</div>");
        let mut in_prose = false;
        for (at, row) in page.rows.iter().enumerate().skip(page.head) {
            if page.blocked_by == Some(at) {
                continue;
            }
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
            } else {
                spans(html, row, Block::Identity, marks, 0);
            }
        }
        if in_prose {
            html.push_str("</span></span>");
        }
        html.push_str("\n</div>");
    }

    /// The head of the bead's page: its status in words beside its id, its
    /// title as the page's heading, and where it is `blocked`, a band naming
    /// each bead it waits on.
    fn head(&self, html: &mut String, node: &Node, blocked: bool) {
        html.push_str(&format!(
            "<p class=\"state\"><span class=\"status{}\">",
            drawn_mark(&node.status).map_or_else(String::new, |mark| format!(" {mark}"))
        ));
        let glyph = show::named(node, Vec::new()).glyph();
        mark(html, &node.status, &glyph, false);
        html.push_str(&format!(
            " {}</span> <span class=\"id\">{}</span></p><h2>{}</h2>",
            escaped(&phrase::status_said(&node.status)),
            escaped(&node.id),
            escaped(&node.title)
        ));
        if blocked {
            self.waits(html, node);
        }
    }

    /// The band naming each bead `node` waits on, each by the reference that
    /// opens it, under what it says of how many there are. A blocker the
    /// answer does not hold is named by its id, and one named twice is
    /// waited on once.
    fn waits(&self, html: &mut String, node: &Node) {
        let mut named = String::new();
        let mut count = 0;
        let by_id = |named: &mut String, blocker: &str| {
            named.push_str(&format!("<span class=\"id\">{}</span>", escaped(blocker)));
        };
        let mut seen = HashSet::new();
        for blocker in node.blocked_by.iter().filter(|id| seen.insert(id.as_str())) {
            let blocking = blocking(self.forest.snapshot(), node, blocker);
            if blocking.is_empty() {
                by_id(&mut named, blocker);
                count += 1;
            }
            for related in &blocking {
                if !self.related(&mut named, related) {
                    by_id(&mut named, blocker);
                }
                count += 1;
            }
        }
        html.push_str(&format!(
            "<div class=\"waits\"><p>{}</p>{named}</div>",
            escaped(&phrase::waiting_on(count))
        ));
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
        self.reference(html, reference, key.map(|key| self.opening(&key)), None);
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
    /// match before and the match after, as `N` and `n` do.
    fn foot(&self, html: &mut String, landed: Option<Landed>, searched: &Asked) {
        let notices = notices(self.forest.snapshot(), &[]);
        if notices.is_empty() && landed.is_none() {
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

/// The name the menu gives `list`. The ready switch is drawn beside the
/// menu, so the ready list goes by the name of every tree there.
fn menu_name(list: Option<&List>) -> String {
    match list {
        Some(List::ToCheck { .. }) => phrase::to_check().to_string(),
        Some(List::LeftToDo { project }) => phrase::left_to_do(project),
        _ => view_of(list).unwrap_or(phrase::every_tree()).to_string(),
    }
}

/// When something was read `at`, as the time of day in `offset`, which the
/// page's script says as how long ago, and shows only once it is old enough
/// to be news.
fn read_time(html: &mut String, at: DateTime<Utc>, offset: FixedOffset, now: DateTime<Utc>) {
    html.push_str(&format!(
        "<span data-shown-once-older-than=\"{}\"> {} <time datetime=\"{}\" \
         title=\"{}\">{}</time></span>",
        READ_SHOWN_ONCE_OLDER_THAN.as_secs(),
        escaped(phrase::read()),
        at.to_rfc3339(),
        escaped(&phrase::instant_in_full(at, offset)),
        escaped(&phrase::at_time_of_day(at, offset, now))
    ));
}

/// How far through something is, `finished` of a `total` above nought, as a
/// bar filled that far beside `said`, which links to `href` where one is
/// given.
fn progress(html: &mut String, finished: usize, total: usize, said: &str, href: Option<&str>) {
    html.push_str(&format!(
        "<span class=\"prog\"><span class=\"bar\" aria-hidden=\"true\">\
         <span style=\"width:{}%\"></span></span>{}</span>",
        finished * 100 / total,
        linked(said, href)
    ));
}

/// `said` as a link to `href`, or as it is where there is none.
fn linked(said: &str, href: Option<&str>) -> String {
    match href {
        Some(href) => format!("<a href=\"{}\">{}</a>", escaped(href), escaped(said)),
        None => escaped(said),
    }
}

/// Spans of one block as HTML, each in its own style, the first of them the
/// block's span `from`. One that `marks` names as a link is a link, where it
/// points somewhere a page may link to, and one it names as a literal is in
/// its colour as written, and one it names as a badge is a pill in the
/// colour its config gave it. Any other span drawn in a status's colour is
/// drawn without it, because a status mark is the one thing the page draws
/// in that colour. One in the code tone is marked as code. A flag in a
/// line's state, drawn in the attention style, holds its words apart from
/// its icon, and is titled with them, so a page short of room can draw the
/// icon alone.
fn spans(html: &mut String, spans: &[Span<'static>], block: Block, marks: &[Mark], from: usize) {
    for (at, span) in spans.iter().enumerate() {
        let mut marked = marks
            .iter()
            .filter(|mark| mark.block == block && mark.at == from + at);
        let badge = marked.clone().any(|mark| matches!(mark.is, Marked::Badge));
        let flagged = (!badge && block == Block::State && span.style == palette::ATTENTION)
            .then(|| span.content.strip_prefix(WARNING))
            .flatten();
        let class = match flagged {
            Some(words) => format!(" class=\"flag\" title=\"{}\"", escaped(words.trim())),
            None if badge => " class=\"pill\"".to_string(),
            None if span.style.fg == palette::CODE.fg => " class=\"code\"".to_string(),
            None => String::new(),
        };
        let css = if marked
            .clone()
            .any(|mark| matches!(mark.is, Marked::Literal))
        {
            palette::css_as_written(span.style)
        } else if badge {
            palette::css(span.style)
        } else {
            palette::css_without_status(span.style)
        };
        let style = format!("{class}{}", styled(css));
        let text = match flagged {
            Some(words) => format!("{WARNING}<span>{}</span>", escaped(words)),
            None => escaped(&span.content),
        };
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
const PHONE_LEVELS: usize = 4;

/// The connectors a line hangs under, with each level drawn as a rail in an
/// element of its own: a trunk as `i`, a gap as `b`, and the line's elbow as
/// `u`, classed by whether it is the last line hung there and whether it
/// hangs on a blocking edge. Under the trees the outermost level is the
/// column a project's roots hang in, which has no rail and is left as text.
/// A level past what a phone indents is classed `deep`. A prefix that is no
/// line's connectors, such as a header's marker, is `None`.
fn railed(prefix: &str, listed: bool) -> Option<String> {
    let (indent, levels) = lines::levels(prefix)?;
    let unrailed = usize::from(!listed);
    let elbow = levels.len() - 1;
    let mut html = escaped(indent);
    for (at, level) in levels.iter().enumerate() {
        let drawn = escaped(level.drawn());
        if at < unrailed {
            html.push_str(&drawn);
            continue;
        }
        let deep = if at == elbow || at - unrailed < PHONE_LEVELS - 1 {
            ""
        } else {
            " class=\"deep\""
        };
        let (tag, class) = match level {
            Level::Trunk => ("i", deep.to_string()),
            Level::Gap => ("b", deep.to_string()),
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
/// A bead's status mark: the shape the stylesheet draws for a status it has
/// one for, or else `glyph`, the status's glyph. Where nothing beside it
/// says the status, the mark says it, as an image named for the status in
/// its colour. Beside the status in words it is hidden from a reader who
/// hears the page, and takes the colour of what holds it.
fn mark(html: &mut String, status: &Status, glyph: &Span, says_the_status: bool) {
    let said = if says_the_status {
        format!(
            " role=\"img\" aria-label=\"{}\"{}",
            escaped(&phrase::status_word(status)),
            styled(palette::css(glyph.style))
        )
    } else {
        " aria-hidden=\"true\"".to_string()
    };
    html.push_str(&format!(
        "<span class=\"g{}\"{said}>{}</span>",
        drawn_mark(status).map_or_else(String::new, |mark| format!(" {mark}")),
        escaped(&glyph.content)
    ));
}

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
/// `--body` for the page, `--display` for a heading, and `--code` for code.
/// Each names its face first and falls back to the reader's own.
const STYLE: &str = "\
:root{--chevron:url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' fill='none' stroke='black' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'><path d='M4 6l4 4 4-4'/></svg>\");\
--body:Inter,system-ui,-apple-system,\"Segoe UI\",sans-serif;\
--display:\"Space Grotesk\",Inter,system-ui,sans-serif;\
--code:\"JetBrains Mono\",ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}\
html{color-scheme:light dark}\
body{margin:0;background:var(--surface);color:var(--fg);font:14px/1.45 var(--body)}\
body{display:flex;flex-direction:column;min-height:100dvh;box-sizing:border-box}main{flex:1 0 auto;align-content:start}\
.code{font-family:var(--code)}\
.code{font-size:.9em}.line .bead .code{font-size:inherit}\
main{display:grid;grid-template-columns:minmax(0,1fr)}\
main.open{position:relative;grid-template-columns:minmax(0,1fr) 500px}\
main.open::before{content:\"\";position:absolute;inset:0 0 0 auto;width:500px;box-sizing:border-box;\
border-left:1px solid var(--line);background:var(--raised);box-shadow:-12px 0 24px -18px rgba(23,32,42,.35)}\
.forest{padding:8px 0;min-width:0;overflow-x:clip}\
.line{display:flex;align-items:center;min-height:32px;white-space:pre;padding:0 12px;box-sizing:border-box;\
position:relative}\
.line>a.bead::after{content:\"\";position:absolute;inset:0}\
.line a:not(.bead){position:relative;z-index:1}\
@media (hover:hover){.line:not(.shown):has(>a.bead):hover{background:var(--sunk)}}\
.line .state{margin-left:auto;padding-left:2ch;flex:none}\
@media (width>720px){.line>.bead{display:flex;align-items:center;gap:10px;min-width:12ch;overflow:hidden;margin-right:auto}\
.line>.bead>*{flex:none}.line>.bead>.title{flex:0 1 auto;min-width:8ch;overflow:hidden;text-overflow:ellipsis}\
.line>.more{display:flex;align-items:center;gap:8px;min-width:0;padding-left:2ch;overflow:hidden}\
.line>.more>.pill{min-width:0;overflow:hidden;text-overflow:ellipsis}\
.line>.bead~.state{margin-left:0}.line>.bead~.state>.prog{width:104px}}\
.pill{display:inline-block;padding:0 8px;border-radius:999px;font:500 11.5px/18px var(--body);\
border:1px solid color-mix(in srgb,currentColor 35%,transparent);background:color-mix(in srgb,currentColor 8%,transparent)}\
.line.shown{background:var(--sel);box-shadow:inset 3px 0 0 var(--accent)}\
#shown{position:absolute;top:0}\
.line>:has(+.bead),.line>.left>.rails{display:inline-flex;align-self:stretch;font-size:0}\
.line>.left:has(>.rails){display:inline-flex;align-self:stretch;align-items:center}\
.line i,.line b,.line u{flex:none;width:24px;position:relative;font:inherit;text-decoration:none}\
.line i::before,.line u::before,.line u::after{content:\"\";position:absolute;left:14px;border:0 solid var(--rail)}\
.line i::before,.line u::before{top:0;bottom:0;border-left-width:1.5px}\
.line u.last::before{bottom:auto;height:var(--arm,16px)}\
.line u::after{top:var(--arm,16px);width:10px;border-top-width:1.5px}\
.line u.blocks::before,.line u.blocks::after{border-style:dashed;border-color:var(--faint)}\
.line>:has(+.bead):not(:has(>.fold))::after,.line .rails:not(:has(>.fold))::after{content:\"\";flex:none;align-self:center;\
width:28px;height:28px;font-size:13px;line-height:28px;text-align:center;color:var(--faint)}\
.line>.rails:has(>u.up):not(:has(>.fold))::after{content:\"\\2191\"}\
.line :has(>a.fold){display:inline-flex;vertical-align:middle;font-size:0}\
.line a.fold{flex:none;align-self:center;display:grid;place-items:center;width:28px;height:28px;\
box-sizing:border-box;border-radius:6px;color:var(--faint)}\
.line a.fold::before{content:\"\";width:14px;height:14px;background:currentColor;\
-webkit-mask:var(--chevron) center/14px no-repeat;mask:var(--chevron) center/14px no-repeat}\
.line a.fold.shut::before{transform:rotate(-90deg)}\
.line a.fold:hover{background:var(--line);color:var(--fg)}\
.line a.fold:focus-visible{outline:2px solid var(--accent);outline-offset:1px}\
a{color:inherit;text-decoration:none}\
a.bead:hover .title,a.close:hover{text-decoration:underline}\
.bead .id{font:500 12px var(--body);color:var(--muted);letter-spacing:.01em}\
.bead .project{padding:0 8px;border:1px solid var(--line);border-radius:999px;font:500 11.5px/18px var(--body);color:var(--muted)}\
aside{position:sticky;top:0;align-self:start;max-height:100vh;overflow:auto;box-sizing:border-box;padding:8px 12px}\
@media (width>=1100px){main.open .line .flag>span{position:absolute;width:1px;height:1px;\
overflow:hidden;clip-path:inset(50%);white-space:nowrap}\
main.open .line .bead>.project{display:none}}\
@media (width<1100px){main.open{grid-template-columns:minmax(0,1fr)}main.open::before{content:none}\
main.open:has(.page)>.forest{display:none}\
aside{position:static;max-height:none;padding:8px 20px 24px}}\
.page{white-space:pre-wrap}\
.page .w{display:none}\
.page .p{display:inline-grid;grid-template-columns:auto minmax(0,1fr);align-items:baseline;\
vertical-align:top;width:100%;white-space:normal}\
.page .lead{white-space:pre}\
.page .p .t{font-size:15px;line-height:1.5;overflow-wrap:anywhere}\
.page .head{display:flex;flex-direction:column;gap:10px;white-space:normal}\
.page .head .state{display:flex;flex-wrap:wrap;align-items:center;gap:10px;margin:0}\
.page .head .state>.id,.waits>.id,.missing>.id{font:500 12.5px var(--body);color:var(--muted);letter-spacing:.01em}\
.status{--status:var(--muted);display:inline-flex;align-items:center;gap:7px;padding:3px 10px 3px 8px;\
border-radius:999px;font:600 12.5px var(--body);color:var(--fg);background:color-mix(in srgb,var(--status) 12%,transparent)}\
.status>.g{color:var(--status)}\
.status.open{--status:var(--status-open)}.status.in-progress{--status:var(--status-in-progress)}\
.status.blocked{--status:var(--status-blocked)}.status.closed{--status:var(--status-closed)}\
.status.deferred{--status:var(--status-deferred)}\
.page .head h2{margin:0;font:600 22px/1.25 var(--display);white-space:pre-wrap;text-wrap:balance;overflow-wrap:anywhere}\
.waits{display:flex;flex-direction:column;gap:6px;padding:10px 12px;border-radius:10px;\
background:var(--sunk);border:1px solid var(--line)}\
.waits>p{margin:0;font:600 11.5px var(--body);letter-spacing:.06em;text-transform:uppercase;color:var(--muted)}\
.close{display:inline-block;margin-bottom:8px}\
html{scroll-padding-top:44px}\
.line.heading{position:sticky;top:0;z-index:2;align-items:center;flex-wrap:wrap;gap:4px 12px;width:auto;min-height:40px;\
padding-block:4px;margin-bottom:4px;background:var(--sunk);border-block:1px solid var(--line);font-size:12.5px;color:var(--muted)}\
.forest>section:not(:first-child){margin-top:18px}\
.heading .left{min-width:0}.heading h2{white-space:normal;overflow-wrap:anywhere}\
.heading h2{display:inline;margin:0;font:600 15px var(--display);color:var(--fg)}\
.heading .state{display:flex;flex:0 1 auto;flex-wrap:wrap;gap:4px 14px;align-items:baseline;padding-left:0}\
.heading .read,.heading .state>*{white-space:nowrap}\
.heading .state a:hover{text-decoration:underline}\
.heading .state a:focus-visible{outline:2px solid var(--accent);outline-offset:1px}\
.mark.ok{color:var(--ok)}.mark.warn,.flag{color:var(--warn)}\
.projects{border-bottom:1px solid var(--line)}\
.projects>summary{list-style:none;cursor:pointer;display:flex;align-items:center;flex-wrap:wrap;gap:4px 12px;\
min-height:40px;padding:4px 12px;box-sizing:border-box;font-size:13px;color:var(--muted)}\
.projects>summary::-webkit-details-marker{display:none}\
.projects .fresh{color:var(--fg);font-weight:600}.projects .fresh>span{color:var(--muted);font-weight:400}\
.projects>summary::after{content:\"\";flex:none;width:13px;height:13px;margin-left:auto;background:var(--accent);\
-webkit-mask:var(--chevron) center/contain no-repeat;mask:var(--chevron) center/contain no-repeat}\
.projects[open]>summary::after{transform:rotate(180deg)}\
.projects>summary:focus-visible{outline:2px solid var(--accent);outline-offset:-2px}\
.projects>section>.line.heading{position:static;margin-bottom:0}\
.prog{display:inline-flex;align-items:center;gap:6px}\
.prog .bar{width:44px;height:5px;border-radius:3px;background:var(--line);overflow:hidden}\
.prog .bar>span{display:block;height:100%;background:var(--ok)}\
footer p,.missing{margin:0}\
footer .said{padding:8px 12px;background:var(--surface)}\
footer .notice{display:flex;align-items:center;gap:10px;padding:9px 20px;font-size:13px;\
background:var(--warn-soft);color:var(--warn);border-top:1px solid var(--line)}\
nav{--tick:url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' fill='none' stroke='black' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'><path d='M3 8.5l3 3 7-7'/></svg>\");\
--lens:url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' fill='none' stroke='black' stroke-width='1.8' stroke-linecap='round'><circle cx='7' cy='7' r='4.5'/><path d='M10.5 10.5L14 14'/></svg>\");\
--list:url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 16 16' fill='none' stroke='black' stroke-width='1.8' stroke-linecap='round'><path d='M3 4h10M3 8h10M3 12h6'/></svg>\");\
display:flex;align-items:center;gap:12px;min-height:56px;box-sizing:border-box;padding:0 20px;\
border-bottom:1px solid var(--line);background:var(--surface)}\
header.title{display:none}\
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
max-height:calc(100dvh - 72px);overflow-y:auto;\
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
nav .check{display:flex;align-items:center;height:30px;padding:0 11px;border-radius:999px;\
background:var(--warn-soft);color:var(--warn);font-weight:600;font-size:13px;white-space:nowrap}\
nav .check.on{box-shadow:inset 0 0 0 1.5px currentColor}\
nav form{flex:0 1 320px;margin-left:auto;display:flex;align-items:center;gap:8px;height:34px;box-sizing:border-box;\
padding:0 12px;border-radius:8px;background:var(--sunk);color:var(--muted)}\
nav button{order:-1;flex:none;display:grid;place-items:center;width:20px;height:20px;padding:0;border:0;border-radius:4px;\
background:none;color:inherit;font-size:0;cursor:pointer}\
nav button::before{--icon:var(--lens);width:16px;height:16px}\
@media (forced-colors:active){nav button{width:auto;font-size:inherit}nav button::before{display:none}\
.line a.fold::before{forced-color-adjust:none;background:CanvasText}\
nav :is(.ready,.check).on{text-decoration:underline}nav form:focus-within{outline:2px solid CanvasText}}\
nav form:focus-within{box-shadow:0 0 0 2px var(--accent)}\
nav label{flex:1;min-width:0;display:flex;align-items:center}\
nav input{flex:1;min-width:0;font:inherit;color:var(--fg);background:none;border:0;padding:0;outline:0}\
nav input::placeholder{color:var(--muted);opacity:1}\
nav label span{display:none}\
nav a:focus-visible,nav summary:focus-visible,nav button:focus-visible{outline:2px solid var(--accent);outline-offset:1px}\
.rooted,nav+.missing{display:flex;flex-wrap:wrap;align-items:center;gap:4px 12px;margin:0;padding:10px 20px;\
border-bottom:1px solid var(--line);font-size:13.5px}\
.rooted{flex-wrap:nowrap;white-space:nowrap;background:var(--accent-soft)}\
.missing{background:var(--warn-soft);color:var(--warn)}aside .missing{padding:10px 12px;border-radius:10px}\
.rooted .said{color:var(--muted)}.rooted .title{font-weight:600}.rooted .bead .id{display:none}\
.rooted .bead{min-width:0;overflow:hidden;text-overflow:ellipsis}\
.all{margin-left:auto;color:var(--accent);font-weight:600;white-space:nowrap}\
.all::before{content:\"\\2715  \"}a.all:hover{text-decoration:underline}\
.root{display:block;margin-bottom:8px}\
a.root:hover,a.step:hover{text-decoration:underline}\
@media (max-width:720px){header.title{display:flex;align-items:baseline;gap:10px;padding:14px 16px 6px}\
header.title h1{flex:1;min-width:0;margin:0;font:600 18px var(--display);overflow-wrap:anywhere}\
header.title>span{font-size:12.5px;color:var(--muted);white-space:nowrap}\
.rooted,nav+.missing{padding:9px 14px}.all::before{content:none}\
aside{padding:8px 16px 24px}\
.line{width:auto;min-height:0;align-items:stretch;white-space:pre-wrap;--arm:9px}\
.line:has(>.bead){display:grid;grid-template-columns:auto 1.6em repeat(4,auto) minmax(0,1fr);\
align-items:baseline;white-space:normal;padding-block:6px;border-bottom:1px solid var(--line)}\
.line>:has(+.bead){grid-area:1/1/3/2;align-self:stretch;margin-block:-6px;--arm:16px}\
.line>:has(+.bead),.line>.left>.rails{display:inline-flex;vertical-align:top;font-size:0;white-space:pre}\
.line>.left:has(>.rails){display:inline-flex;align-items:flex-start;max-width:100%}\
.line>.left>.rails+span{min-width:0}\
.line i,.line b,.line u{width:14px}.line .deep{display:none}\
.line i::before,.line u::before,.line u::after{left:10px}\
.line>:has(+.bead):not(:has(>.fold))::after,.line .rails:not(:has(>.fold))::after\
{align-self:flex-start;width:32px;height:20px;line-height:20px}\
.line a.fold{align-self:flex-start;width:32px;height:44px}\
.line>:has(+.bead)>a.fold{place-items:start center;padding-top:9px}\
.line:has(>.left a.fold){--arm:22px}.line>.left:has(a.fold){align-items:center}\
.line>:has(+.bead)::after{margin-top:6px}\
.line>.bead{grid-area:1/2/3/-1;display:grid;grid:subgrid/subgrid;align-items:baseline}\
.line .g{grid-area:1/1;align-self:start;font-size:14px;text-align:center}\
.line .g:is(.open,.in-progress,.blocked,.closed,.deferred){margin-top:4px}\
.line .title{grid-area:1/2/2/-1;min-width:10em;font-size:15px;line-height:1.35;overflow-wrap:anywhere}\
.line>.bead>.id,.line>.more,.line>.bead~.state{grid-row:2;font-size:12px}\
.line>.bead>.id{grid-column:2}.line>.bead>:not(.title){font-family:var(--body)}\
.line>.bead>.project{grid-area:2/3;justify-self:start;margin-left:8px;overflow-wrap:anywhere}\
.line>.more{grid-column:5;padding-left:2ch;overflow-wrap:anywhere}\
.line>.bead~.state{grid-column:6;margin-left:0;overflow-wrap:anywhere}.line>.bead~.state>.prog>.bar{display:none}\
.line>.bead~.state>.flag{display:inline-block;vertical-align:top}\
nav{order:1;position:sticky;bottom:0;z-index:2;min-height:0;gap:6px;\
padding:8px 10px calc(8px + env(safe-area-inset-bottom));border-bottom:0;border-top:1px solid var(--line);\
font-size:13.5px}\
nav>*,nav>form{flex:1 1 0;min-width:0;margin:0}nav>.lists{flex-grow:1.3;position:static}nav>:is(.brand,.check){display:none}\
.lists summary,nav>.ready,nav form{justify-content:center;gap:6px;height:48px;padding:0 4px;border:0;border-radius:10px;\
background:none;color:var(--muted);font-weight:500;white-space:nowrap;overflow:hidden}\
.lists[open] summary{box-shadow:none}\
nav .ready::before{width:26px;height:16px;\
background:radial-gradient(circle at 8px 50%,var(--surface) 6px,transparent 6.5px),var(--rail)}\
nav .ready.on::before{background:radial-gradient(circle at calc(100% - 8px) 50%,var(--surface) 6px,transparent 6.5px),var(--accent)}\
nav .ready.on,nav:not(:has(.ready.on))>.lists>summary,.lists[open] summary\
{background:var(--accent-soft);color:var(--accent);font-weight:600}\
nav form:focus-within{box-shadow:none}nav input:focus-visible{box-shadow:0 0 0 2px var(--accent)}\
nav button{display:none}nav button::before{display:none}\
nav label{justify-content:center;gap:6px;height:100%;cursor:text}\
nav label::before,.lists summary>span::before{content:\"\";flex:none;width:16px;height:16px;background:currentColor;\
-webkit-mask:var(--icon) center/contain no-repeat;mask:var(--icon) center/contain no-repeat}\
nav label::before{--icon:var(--lens)}\
.lists summary>span::before{--icon:var(--list);display:inline-block;margin-right:6px;vertical-align:-3px}\
nav input{flex:none;width:0}nav label span{display:inline}\
nav:has(form:focus-within,input[type=search]:is(:not(:placeholder-shown),:not([value='']))) label span{display:none}\
nav:has(form:focus-within){height:0;padding-block:0;border:0}nav:has(form:focus-within)>:not(form){visibility:hidden}\
nav:has(form:focus-within,input[type=search]:is(:not(:placeholder-shown),:not([value='']))) form{position:fixed;top:0;left:0;right:0;display:flex;gap:8px;height:auto;padding:8px 12px;border-radius:0;\
background:var(--surface);border-bottom:1px solid var(--line)}\
html:has(nav form:focus-within,nav input[type=search]:is(:not(:placeholder-shown),:not([value='']))){scroll-padding-top:116px}body:has(nav form:focus-within,nav input[type=search]:is(:not(:placeholder-shown),:not([value='']))){padding-top:52px}\
body:has(nav form:focus-within,nav input[type=search]:is(:not(:placeholder-shown),:not([value='']))) .line.heading{top:52px}\
nav:has(form:focus-within,input[type=search]:is(:not(:placeholder-shown),:not([value='']))) input{flex:1;width:auto;padding:7px 8px;border-radius:8px;text-align:left;background:var(--sunk);font-size:16px;font-weight:normal}\
footer a.step{display:inline-block;padding:0 10px;font-size:18px}\
footer .notice{padding:9px 16px}\
html{scroll-padding-top:64px}.line.heading{gap:4px 10px;margin-bottom:0}\
nav:has(form:focus-within,input[type=search]:is(:not(:placeholder-shown),:not([value='']))) button{display:block;order:0;width:auto;height:auto;background:none;color:var(--accent);font-size:16px}\
.lists>div{display:none}\
.lists[open]>div{display:block;position:absolute;top:auto;left:0;right:0;bottom:100%;width:auto;padding:8px 10px 16px;\
max-height:calc(100dvh - 74px - env(safe-area-inset-bottom));overflow-y:auto;border:0;border-radius:18px 18px 0 0}\
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
.waits>.bead{width:100%}}\
:is(.bead,.status) .g:is(.open,.in-progress,.blocked,.closed,.deferred){display:inline-block;position:relative;\
box-sizing:border-box;width:12px;height:12px;border:1.75px solid currentColor;border-radius:50%;\
font-size:0;overflow:hidden;vertical-align:-1px;justify-self:center}\
.bead .g.open{color:var(--status-open)}\
:is(.bead,.status) .g.in-progress{background:linear-gradient(90deg,currentColor 50%,transparent 50%)}\
:is(.bead,.status) .g:is(.blocked,.closed){background:currentColor}\
:is(.bead,.status) .g.closed::after{content:\"\";position:absolute;left:2.6px;top:.6px;width:3px;height:6px;\
border:solid var(--surface);border-width:0 1.6px 1.6px 0;transform:rotate(45deg)}\
:is(.bead,.status) .g.deferred{border-style:dashed}\
@media print,(forced-colors:active){:is(.bead,.status) .g:is(.open,.in-progress,.blocked,.closed,.deferred){\
width:auto;height:auto;margin-top:0;border:0;border-radius:0;background:none;font-size:inherit;overflow:visible}\
:is(.bead,.status) .g.closed::after{content:none}}";

/// How old a project's read is before its heading says when it was. A
/// project kept current is read or vouched for well inside this, by its
/// poll or its producer's heartbeat, so its age only distracts. Past this,
/// the age is news.
const READ_SHOWN_ONCE_OLDER_THAN: std::time::Duration = std::time::Duration::from_secs(120);

/// The part of the page's script that says each time on the page as the
/// reader would: how long ago, in the page's language, and the instant in
/// full on hover, in the reader's own locale and zone. A request never says which zone its reader is in, so the server
/// cannot. It says them as the page opens, on each page that keeps it
/// current, and again as they age, from the browser's clock alone. A
/// browser that cannot say an age keeps the time the server drew, and so
/// does one whose clock is behind the server's, which would say a stale
/// read was fresh.
///
/// A time inside an element naming how old it must be to be shown hides
/// that element until it is that old. The server draws it shown, so a
/// browser running no script keeps saying it. A time the browser's clock
/// puts less than that far in the future stays hidden too, because a clock
/// a second behind the server's is common. Further out, the clock is too
/// wrong to judge by, and the time is shown.
const IN_READERS_ZONE: &str = r##"const LARGER = [["minute", 60], ["hour", 60], ["day", 24]];
const inReadersZone = (on) => {
  const says = window.Intl?.RelativeTimeFormat;
  const ago = says && new Intl.RelativeTimeFormat(document.documentElement.lang, { numeric: "always" });
  const inFull = says && new Intl.DateTimeFormat(undefined, { dateStyle: "long", timeStyle: "long" });
  for (const time of on.querySelectorAll("time[datetime]")) {
    const at = new Date(time.getAttribute("datetime"));
    let since = (Date.now() - at) / 1000, unit = "second";
    const shown = time.closest("[data-shown-once-older-than]");
    if (shown) shown.hidden = Math.abs(since) < Number(shown.dataset.shownOnceOlderThan);
    if (!says || !(since >= 0)) continue;
    for (const [next, per] of LARGER) {
      if (since < per) break;
      since /= per;
      unit = next;
    }
    time.textContent = ago.format(-Math.floor(since), unit);
    time.title = inFull.format(at);
  }
};
addEventListener("DOMContentLoaded", () => inReadersZone(document));
setInterval(() => document.hidden || inReadersZone(document), 15000);
document.addEventListener("visibilitychange", () => document.hidden || inReadersZone(document));
"##;

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
  const boxes = () =>
    [document.querySelector(".forest"), document.querySelector("aside"), document.querySelector(".lists>div")];
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
    const top = before.findIndex((at) => !at.matches(".heading") && at.getBoundingClientRect().bottom > 0);
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
    inReadersZone(drawn);
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

/// The part of the page's script that closes an open bead on Escape, as
/// its close link does. Escape is left to a field being typed in, and closes
/// an open menu before the bead.
const ESCAPE_CLOSES: &str = r##"addEventListener("keydown", (event) => {
  if (event.key !== "Escape" || event.defaultPrevented || event.target.matches?.("input,textarea,select")) return;
  const menu = document.querySelector("details[open]");
  if (menu) {
    menu.open = false;
    menu.querySelector("summary").focus();
    return;
  }
  const close = document.querySelector("aside>a.close");
  if (close) location.assign(close.href);
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
    const READ: &str = "✓ read at 11:21";

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
                    .replace("<span class=\"flag\">", " ")
                    .replace("<a class=\"flag\"", " <a class=\"flag\"");
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
    /// the project's line said as the page's heading says it: the time of
    /// day it was read, and the fraction done as what is left in words. A run
    /// of finished beads and a row's flag are said as the page says them too.
    fn terminal_lines(forest: &Forest) -> Vec<String> {
        let rows = frame_of(forest, 160, 60).rows();
        rows[..forest.lines().len()]
            .iter()
            .zip(forest.lines().viewport(0, forest.lines().len()))
            .map(|(row, (_, line))| {
                let row = words(row);
                match &line.content {
                    Content::Elided { count, .. } => {
                        return row.replace(&phrase::elided(*count), &phrase::finished_run(*count));
                    }
                    Content::Bead(bead) | Content::Waiting(Waiting { row: bead, .. }) => {
                        if let Some(flag) = crate::view::row::anomaly_marker(&bead.anomalies) {
                            let said = phrase::flag(&bead.anomalies).expect("a rule fired");
                            return row.replace(&flag, &format!("{WARNING} {said}"));
                        }
                    }
                    _ => {}
                }
                if !row.contains(AGO) {
                    return row;
                }
                row.replace(AGO, READ)
                    .split(' ')
                    .map(|word| match word.split_once('/') {
                        Some((done, of)) => {
                            let [done, of] =
                                [done, of].map(|n| n.parse::<usize>().expect("a count"));
                            phrase::left_of(of - done, of)
                        }
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

    /// A project's heading says the time of day it was read behind the same
    /// mark as the terminal, and says how much of it is left beside a bar
    /// filled by the share that is done.
    #[test]
    fn a_projects_heading_says_the_time_of_day_it_was_read_where_the_terminal_says_how_long_ago() {
        let terminal = frame_of(&terminal_forest(&[]), 160, 60).rows();
        assert_eq!(words(&terminal[0]), format!("▾ summit-works {AGO} 3/9"));
        let html = page_at("/").html;
        assert_eq!(
            page_lines(&html)[0],
            format!("▾ summit-works {READ} 6 left of 9")
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

    /// The short time keeps the whole of the read, day and zone included,
    /// for a reader whose page has been open since another day.
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
        assert!(full.starts_with("11:21:"), "{full}");
        assert!(full.contains(" on "), "{full}");
        assert!(time.starts_with("<time datetime=\""), "{time}");
    }

    /// A heading's read time stands apart from its mark, telling the page's
    /// script how old it has to be before the reader is shown it.
    #[test]
    fn a_headings_read_time_says_how_old_it_must_be_to_be_shown() {
        let html = page_at("/").html;
        let heading = line_html(&html, 0);
        let shown = "<span data-shown-once-older-than=\"120\"> read <time datetime=\"";
        assert!(heading.contains(shown), "{heading}");
    }

    /// The title over a phone's list, from its opening tag to its close.
    fn title_over(html: &str) -> &str {
        let from = html.find("<header class=\"title\">").expect("a title");
        &html[from..from + html[from..].find("</header>").expect("the title ends")]
    }

    /// A phone's page is headed by the name of the list it draws.
    #[test]
    fn a_phone_page_is_titled_for_the_list_it_draws() {
        let trees = page_at("/").html;
        let ready = page_at(&link_saying(&trees, phrase::ready_only())).html;
        let view = page_at(&link_saying(&trees, "theming")).html;
        let to_check = checked_at(&link_saying(&trees, phrase::to_check()));
        let left = page_at(&link_saying(line_html(&trees, 0), &phrase::left_of(6, 9))).html;
        let left_title = escaped(&phrase::left_to_do(PROJECT));
        for (html, title) in [
            (&trees, "All trees"),
            (&ready, "Ready"),
            (&view, "theming"),
            (&to_check, "To check"),
            (&left, left_title.as_str()),
        ] {
            assert!(
                title_over(html).contains(&format!("<h1>{title}</h1>")),
                "{}",
                title_over(html)
            );
        }
    }

    /// Beside its title a phone's page says when the oldest of its reads
    /// was, the one claim true of every row, once it is old enough to be
    /// news, as a project's heading does.
    #[test]
    fn a_phone_pages_title_says_its_oldest_read_once_that_is_old_enough_to_show() {
        let oldest = drawn_at() - chrono::TimeDelta::hours(1);
        let mut snapshot = ground();
        snapshot.read_at.insert(PROJECT.to_string(), oldest);
        let asked = Asked::of("/").expect("the forest");
        let html = page(snapshot, &Layout::default(), &asked, drawing()).html;
        let title = title_over(&html);
        let shown = format!(
            "<span data-shown-once-older-than=\"120\"> read <time datetime=\"{}\"",
            oldest.to_rfc3339()
        );
        assert!(title.contains(&shown), "{title}");
    }

    /// A project the page does not draw has no say in its title's read
    /// time, however long ago it was read.
    #[test]
    fn a_phone_pages_title_ignores_a_read_of_a_project_it_does_not_draw() {
        let long_ago = drawn_at() - chrono::TimeDelta::hours(1);
        let mut snapshot = ground();
        snapshot.read_at.insert("tidewater".to_string(), long_ago);
        let asked = Asked::of("/").expect("the forest");
        let html = page(snapshot, &Layout::default(), &asked, drawing()).html;
        let title = title_over(&html);
        assert!(title.contains("<time datetime=\""), "{title}");
        assert!(!title.contains(&long_ago.to_rfc3339()), "{title}");
    }

    /// While a project the page draws has yet to be read, no read time is
    /// true of every row, so the title gives none.
    #[test]
    fn a_phone_pages_title_gives_no_read_time_while_a_project_it_draws_is_unread() {
        let mut snapshot = ground();
        snapshot.projects.push("tidewater".to_string());
        let asked = Asked::of("/").expect("the forest");
        let html = page(snapshot, &Layout::default(), &asked, drawing()).html;
        assert!(html.contains("<h2>tidewater</h2>"), "{html}");
        let title = title_over(&html);
        assert!(!title.contains("<time"), "{title}");
    }

    /// A bead's page has no list title over it.
    #[test]
    fn a_beads_page_has_no_list_title() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        assert!(!html.contains("<header"), "{html}");
    }

    /// The title heads a phone's page and nothing wider, set in the display
    /// face at the size a phone sets a bead's title.
    #[test]
    fn the_list_title_is_drawn_on_a_phone_alone() {
        assert!(STYLE.contains("header.title{display:none}"), "{STYLE}");
        let phone = rules_under("(max-width:720px)");
        assert!(phone.contains("header.title{display:flex;"), "{phone}");
        assert!(
            phone.contains(
                "header.title h1{flex:1;min-width:0;margin:0;font:600 18px var(--display)"
            ),
            "{phone}"
        );
    }

    /// A phone's foot bar is 48 pixels tall, and each of its controls
    /// carries an icon: the menu a list, the switch its knob and the search
    /// its lens.
    #[test]
    fn a_phones_foot_bar_controls_are_48_pixels_tall_with_an_icon() {
        let phone = rules_under("(max-width:720px)");
        assert!(
            phone.contains(
                ".lists summary,nav>.ready,nav form{justify-content:center;gap:6px;height:48px;"
            ),
            "{phone}"
        );
        assert!(
            phone.contains(".lists summary>span::before{--icon:var(--list);"),
            "{phone}"
        );
        assert!(
            phone.contains("nav label::before{--icon:var(--lens)}"),
            "{phone}"
        );
        assert!(STYLE.contains("--list:url("), "{STYLE}");
    }

    /// The list menu opens above the foot bar and stops short of the top of
    /// the screen: the bar is its 48 pixel controls, 8 pixels of padding
    /// either side, a 1 pixel border and the screen's inset at the bottom.
    #[test]
    fn a_phones_list_menu_fits_above_the_foot_bar() {
        let phone = rules_under("(max-width:720px)");
        assert!(
            phone.contains("max-height:calc(100dvh - 74px - env(safe-area-inset-bottom))"),
            "{phone}"
        );
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

    /// A page drawn again a minute later, from the same reads, is the same
    /// page, so a page kept current is sent nothing for the time passing.
    #[test]
    fn a_page_drawn_again_later_the_same_day_is_the_same_page() {
        let later = Drawn {
            at: drawn_at() + chrono::TimeDelta::minutes(1),
            ..drawing()
        };
        let asked = Asked::of("/").expect("the forest");
        assert_eq!(
            page(ground(), &Layout::default(), &asked, later).html,
            page_at("/").html
        );
    }

    /// The beads in a project to check are said in its heading, in the
    /// warning's colour, as a link to the To check view narrowed to the
    /// project, which lists exactly those beads.
    #[test]
    fn a_headings_beads_to_check_link_to_them_in_the_to_check_view() {
        let html = checked_at("/");
        assert_eq!(
            page_lines(&html)[0],
            format!("▾ summit-works {READ} 6 left of 9 {WARNING} 1 bead to check")
        );
        let heading = line_html(&html, 0);
        assert!(heading.contains("<a class=\"flag\" href="), "{heading}");

        let to = link_saying(heading, &format!("{WARNING} 1 bead to check"));
        assert_eq!(
            Asked::of(&to).expect("a page is at the address").list,
            Some(List::ToCheck {
                project: Some(PROJECT.to_string())
            })
        );
        assert_eq!(under_the_heading(&checked_at(&to)), checking(Some(PROJECT)));
    }

    /// The terminal's lines, set apart as `as_both_draw` sets them, with the
    /// ground drawing the beads left to do in its project.
    fn left_to_do() -> Vec<String> {
        let mut forest = flatten(ground());
        forest.draw_list(Some(List::LeftToDo {
            project: PROJECT.to_string(),
        }));
        as_both_draw(terminal_lines(&forest))
    }

    /// What a project's heading says is left is a link to a list of exactly
    /// those beads, named for the project.
    #[test]
    fn a_headings_count_of_what_is_left_links_to_those_beads() {
        let html = page_at("/").html;
        let to = link_saying(line_html(&html, 0), &phrase::left_of(6, 9));
        assert_eq!(
            Asked::of(&to).expect("a page is at the address").list,
            Some(List::LeftToDo {
                project: PROJECT.to_string()
            })
        );

        let left = page_at(&to).html;
        assert_eq!(under_the_heading(&left), left_to_do());
        let section = phrase::section(
            &lines::Section::LeftToDo {
                project: PROJECT.to_string(),
            },
            6,
        );
        assert!(page_lines(&left).contains(&section), "{left}");
        assert!(
            left.contains(&format!(
                "<summary><span>{}</span>",
                escaped(&phrase::left_to_do(PROJECT))
            )),
            "{left}"
        );
        assert!(!inside(&left, "lists").contains(" class=\"on\""), "{left}");
    }

    /// A heading with nothing to check links only to what is left.
    #[test]
    fn a_heading_with_nothing_to_check_has_only_the_one_link() {
        let html = page_at("/").html;
        let state = inside(line_html(&html, 0), "state");
        assert_eq!(state.matches("<a ").count(), 1, "{state}");
    }

    /// Rooted at one bead, the heading counts that tree, and its link lists
    /// what is left of that tree under a heading counting the same.
    #[test]
    fn a_heading_over_one_tree_links_to_what_is_left_of_that_tree() {
        let rooted = page_at(&link_saying(
            &page_at(&address(&key("smt-4kd3p.6"))).html,
            phrase::only_this_tree(),
        ))
        .html;
        let left = page_at(&link_saying(line_html(&rooted, 0), &phrase::left_of(3, 3))).html;
        let heading = &page_lines(&left)[0];
        assert!(heading.ends_with(&phrase::left_of(3, 3)), "{heading}");
        let section = phrase::section(
            &lines::Section::LeftToDo {
                project: PROJECT.to_string(),
            },
            3,
        );
        assert!(page_lines(&left).contains(&section), "{left}");
    }

    /// An address keeps the list of what is left to one project, whatever
    /// its name holds, and reads back as that list.
    #[test]
    fn the_beads_left_to_do_in_one_project_have_an_address_that_reads_back() {
        for project in [PROJECT, "dock & yard/east #2"] {
            let list = Some(List::LeftToDo {
                project: project.to_string(),
            });
            let address = Asked::default().listing(list.clone());
            let asked = Asked::of(&address).expect("a page is at the address");
            assert_eq!(asked.list, list, "{address}");
        }
        assert_eq!(Asked::of("/?left=1").expect("the forest").list, None);
    }

    /// A row's flag holds its icon apart from its words, so a page with a
    /// bead open beside the trees can draw the icon alone, and the flag keeps
    /// its words as its title.
    #[test]
    fn a_rows_flag_holds_its_words_apart_from_its_icon() {
        let checked = with_the_bead_changed(|node| {
            node.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        });
        let html = page_of(checked, &address(&key("smt-4kd3p.6")));
        let row = lines_html(&html)
            .into_iter()
            .find(|line| text_of(line).contains("smt-4kd3p.6"))
            .expect("the bead has a row");
        let open = row
            .find("<span class=\"flag\"")
            .expect("the row has a flag");
        let flag = &row[open..element_end(row, open)];
        let title = flag.split("title=\"").nth(1).expect("the flag is titled");
        let title = &title[..title.find('"').expect("the title ends")];
        assert!(!title.is_empty(), "{flag}");
        assert!(
            flag.contains(&format!(">{WARNING}<span> {title}</span></span>")),
            "{flag}"
        );
    }

    /// The row of `id` on the page drawn at the bead's address.
    fn row_of(snapshot: Snapshot, id: &str) -> String {
        let html = page_of(snapshot, &address(&key(id)));
        lines_html(&html)
            .into_iter()
            .find(|line| text_of(line).contains(id))
            .expect("the bead has a row")
            .to_string()
    }

    /// Every rule that fired is said once, as one sentence, where the
    /// terminal joins a phrase for each.
    #[test]
    fn a_rows_flag_says_every_rule_that_fired_as_one_sentence() {
        let fired = vec![
            Anomaly::StaleClaim { days: 58 },
            Anomaly::OrphanClaim { refused: None },
        ];
        let checked = with_the_bead_changed(|node| node.anomalies = fired.clone());
        let row = row_of(checked, "smt-4kd3p.6");
        let said = phrase::flag(&fired).expect("two rules fired");

        assert_eq!(row.matches("class=\"flag\"").count(), 1, "{row}");
        assert!(
            row.contains(&format!("<span class=\"flag\" title=\"{said}\"")),
            "{row}"
        );
        assert!(
            row.contains(&format!(">{WARNING}<span> {said}</span></span>")),
            "{row}"
        );
        assert!(!text_of(&row).contains(" · "), "{row}");
    }

    /// An agent on a bead is a chip naming it, and an agent watching it is
    /// one too where none is on it. A bead with both has the one chip.
    #[test]
    fn a_row_names_an_agent_on_its_bead_in_a_chip() {
        use crate::model::watching::Watching;
        use crate::view::draw::tests::a_pane;

        let chips = |row: &str| -> Vec<String> {
            row.match_indices("<span class=\"pill\"")
                .map(|(at, _)| text_of(&row[at..element_end(row, at)]))
                .filter(|said| {
                    [phrase::agent_chip(), phrase::watched_chip()].contains(&said.as_str())
                })
                .collect()
        };
        let watching = Watching::Known {
            agents: Vec::new(),
            unreported: 1,
        };

        let staffed = row_of(
            with_the_bead_changed(|node| node.agent = Some(a_pane())),
            "smt-4kd3p.6",
        );
        assert_eq!(chips(&staffed), [phrase::agent_chip()], "{staffed}");
        assert!(
            !text_of(&staffed).contains(crate::view::row::AGENT),
            "{staffed}"
        );

        let watched = row_of(
            with_the_bead_changed(|node| node.watching = watching.clone()),
            "smt-4kd3p.6",
        );
        assert_eq!(chips(&watched), [phrase::watched_chip()], "{watched}");
        assert!(
            !text_of(&watched).contains(crate::view::row::WATCHED),
            "{watched}"
        );

        let both = row_of(
            with_the_bead_changed(|node| {
                node.agent = Some(a_pane());
                node.watching = watching.clone();
            }),
            "smt-4kd3p.6",
        );
        assert_eq!(chips(&both), [phrase::agent_chip()], "{both}");

        let nobody = row_of(ground(), "smt-4kd3p.6");
        assert!(chips(&nobody).is_empty(), "{nobody}");
    }

    /// The chip and the flag are drawn in the blocks the layout puts the
    /// agent and the anomalies in.
    #[test]
    fn a_rows_chip_and_flag_keep_the_places_the_layout_gives_them() {
        use crate::view::draw::tests::a_pane;

        let layout = Layout {
            title: vec![Cell::Title, Cell::Agent],
            state: vec![Cell::Progress, Cell::Anomalies],
            ..Layout::default()
        };
        let checked = with_the_bead_changed(|node| {
            node.agent = Some(a_pane());
            node.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        });
        let asked = Asked::of(&address(&key("smt-4kd3p.6"))).expect("a page");
        let html = page(checked, &layout, &asked, drawing()).html;
        let row = lines_html(&html)
            .into_iter()
            .find(|line| text_of(line).contains("smt-4kd3p.6"))
            .expect("the bead has a row");

        let more = row.find("<span class=\"more\">").expect("a title block");
        let state = row.find("<span class=\"state\">").expect("a state");
        let chip = row
            .find(&format!(">{}</span>", phrase::agent_chip()))
            .expect("a chip");
        let flag = row.find("<span class=\"flag\"").expect("a flag");
        assert!(more < chip && chip < state, "{row}");
        assert!(state < flag, "{row}");
    }

    /// A shut run of finished beads says how many, and nothing about agents.
    #[test]
    fn a_run_of_finished_beads_says_how_many_finished() {
        let html = page_at("/").html;
        let runs: Vec<String> = lines_html(&html)
            .into_iter()
            .map(text_of)
            .filter(|text| text.contains("finished"))
            .collect();

        assert_eq!(runs.len(), 1, "{html}");
        assert!(
            runs[0].trim_end().ends_with(&phrase::finished_run(3)),
            "{runs:?}"
        );
    }

    /// A badge whose own words open with the warning sign is not a flag, so
    /// its words stay drawn beside an open bead.
    #[test]
    fn a_badge_worded_like_a_flag_keeps_its_words() {
        let mut html = String::new();
        let badge = Span::raw(format!("{WARNING} Needs review"));
        spans(&mut html, &[badge], Block::State, &[], 0);
        assert!(!html.contains("flag"), "{html}");
    }

    /// The rules the page's style sets under the media query `query`.
    fn rules_under(query: &str) -> &'static str {
        let opened = format!("@media {query}{{");
        let from = STYLE.find(&opened).expect("the style has the query") + opened.len();
        let mut depth = 1;
        for (at, c) in STYLE[from..].char_indices() {
            depth += match c {
                '{' => 1,
                '}' => -1,
                _ => 0,
            };
            if depth == 0 {
                return &STYLE[from..from + at];
            }
        }
        panic!("the query's rules end")
    }

    /// On a desktop a bead opens in a 500 pixel panel beside the trees,
    /// which take the rest. Narrower than that, the bead takes the whole
    /// page and the trees are not drawn beside it.
    #[test]
    fn a_bead_opens_beside_the_trees_on_a_desktop_and_takes_the_page_below_it() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        assert!(
            html.contains("<main class=\"open\"><div class=\"forest\">"),
            "{html}"
        );
        assert!(
            STYLE
                .contains("main.open{position:relative;grid-template-columns:minmax(0,1fr) 500px}"),
            "{STYLE}"
        );
        let narrower = rules_under("(width<1100px)");
        for rule in [
            "main.open{grid-template-columns:minmax(0,1fr)}",
            "main.open::before{content:none}",
            "main.open:has(.page)>.forest{display:none}",
        ] {
            assert!(narrower.contains(rule), "{narrower}");
        }
    }

    /// With a bead open beside the trees, a row's flag draws its icon alone,
    /// and its words stay for a screen reader.
    #[test]
    fn a_flag_shrinks_to_its_icon_beside_an_open_bead() {
        let beside = rules_under("(width>=1100px)");
        assert!(
            beside.contains(
                "main.open .line .flag>span{position:absolute;width:1px;height:1px;\
                 overflow:hidden;clip-path:inset(50%);white-space:nowrap}"
            ),
            "{beside}"
        );
    }

    /// The row of the bead open is marked by the selection's tint and an
    /// edge in the accent.
    #[test]
    fn the_row_of_the_bead_open_is_marked_by_an_edge() {
        assert!(
            STYLE.contains(
                ".line.shown{background:var(--sel);box-shadow:inset 3px 0 0 var(--accent)}"
            ),
            "{STYLE}"
        );
    }

    /// Escape follows the open bead's close link, which is the one the page
    /// draws in the panel.
    #[test]
    fn escape_follows_the_link_that_closes_the_bead() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        assert!(html.contains(ESCAPE_CLOSES), "{html}");
        assert!(ESCAPE_CLOSES.contains("querySelector(\"aside>a.close\")"));
        assert!(html.contains("<aside><a class=\"close\" href=\""), "{html}");
    }

    /// A menu does not close itself on Escape, so Escape closes an open
    /// menu first, and the next Escape closes the bead.
    #[test]
    fn escape_closes_an_open_menu_before_the_bead() {
        let menu = ESCAPE_CLOSES
            .find("menu.open = false")
            .expect("Escape closes the menu");
        let close = ESCAPE_CLOSES
            .find("location.assign")
            .expect("Escape closes the bead");
        assert!(menu < close, "{ESCAPE_CLOSES}");
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

    fn checked() -> Snapshot {
        with_the_bead_changed(|node| {
            node.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        })
    }

    fn checked_at(target: &str) -> String {
        let asked = Asked::of(target).expect("a page is at the address");
        page(checked(), &Layout::default(), &asked, drawing()).html
    }

    /// The terminal's lines, set apart as `as_both_draw` sets them, with the
    /// checked ground drawing the beads to check in `project`, or in every
    /// project.
    fn checking(project: Option<&str>) -> Vec<String> {
        let mut forest = flatten(checked());
        forest.draw_list(Some(List::ToCheck {
            project: project.map(str::to_string),
        }));
        as_both_draw(terminal_lines(&forest))
    }

    fn under_the_heading(html: &str) -> Vec<String> {
        as_both_draw(page_lines(html))
    }

    /// `lines` without the two things the page draws apart from the
    /// terminal: the project's heading, which it words apart, and each row's
    /// progress, which it draws last, after the flags.
    fn as_both_draw(lines: Vec<String>) -> Vec<String> {
        let progress = |word: &str| {
            word.split_once('/').is_some_and(|(done, of)| {
                [done, of]
                    .iter()
                    .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            })
        };
        lines
            .into_iter()
            .filter(|line| !line.starts_with(PROJECT))
            .map(|line| {
                line.split(' ')
                    .filter(|word| !progress(word))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    /// The beads to check are a list of their own. The menu counts them and
    /// a button in the top bar says how many, and each lists them in one
    /// step. No list says them again at the foot of the page.
    #[test]
    fn the_menu_and_the_top_bar_each_list_the_beads_to_check_with_their_count() {
        let said = format!("{WARNING} {}", phrase::anomalies_beneath(1));
        let ready = checked_at(&link_saying(&page_at("/").html, phrase::ready_only()));
        assert_eq!(count_beside(&ready, phrase::to_check()), "1");
        assert!(ready.contains("<a class=\"check\" href="), "{ready}");
        assert!(!ready.contains("<footer>"), "{ready}");

        let from_menu = checked_at(&link_saying(&ready, phrase::to_check()));
        let from_button = checked_at(&link_saying(&ready, &said));
        assert_eq!(from_button, from_menu);
        assert_eq!(under_the_heading(&from_menu), checking(None));
        assert!(
            page_lines(&from_menu)
                .iter()
                .any(|line| line.contains("apply the two niri settings")),
            "{:#?}",
            page_lines(&from_menu)
        );
        assert!(from_menu.contains("<a class=\"check on\""), "{from_menu}");
        assert!(!from_menu.contains("<a class=\"ready on\""), "{from_menu}");
        assert!(!from_menu.contains("<footer>"), "{from_menu}");
    }

    /// With nothing to check the menu still offers the list, counting none,
    /// and the top bar has no button for it.
    #[test]
    fn with_nothing_to_check_the_top_bar_has_no_button_for_it() {
        let html = page_at("/").html;
        assert_eq!(count_beside(&html, phrase::to_check()), "0");
        assert!(!html.contains("class=\"check"), "{html}");
    }

    /// The menu names the list it is open on, and marks only that entry.
    #[test]
    fn the_menu_names_the_beads_to_check_when_they_are_listed() {
        let html = checked_at(&link_saying(&page_at("/").html, phrase::to_check()));
        assert!(
            html.contains(&format!("<summary><span>{}</span>", phrase::to_check())),
            "{html}"
        );
        let menu = inside(&html, "lists");
        assert_eq!(menu.matches(" class=\"on\"").count(), 1, "{menu}");
        assert!(
            menu.contains(&format!(" class=\"on\">{}<span", phrase::to_check())),
            "{menu}"
        );
    }

    /// An address can keep the list to one project, whatever its name holds,
    /// and the page then lists what the terminal's list kept to it holds.
    #[test]
    fn the_beads_to_check_in_one_project_have_an_address_that_reads_back() {
        for project in [PROJECT, "dock & yard/east #2"] {
            let list = Some(List::ToCheck {
                project: Some(project.to_string()),
            });
            let address = Asked::default().listing(list.clone());
            let asked = Asked::of(&address).expect("a page is at the address");
            assert_eq!(asked.list, list, "{address}");
            let html = checked_at(&address);
            assert_eq!(
                under_the_heading(&html),
                checking(Some(project)),
                "{address}"
            );
            // The menu and the button list every project's beads to check,
            // which is not this page.
            assert!(!inside(&html, "lists").contains(" class=\"on\""), "{html}");
            assert!(!html.contains("<a class=\"check on\""), "{html}");
        }
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
            .position(|line| line.contains(&format!("✓ {}", phrase::finished_run(3))))
            .expect("the finished siblings are one run");

        let opened = page_at(&fold_link(&page_at("/").html, run)).html;
        assert_eq!(
            page_lines(&opened),
            terminal_lines(&terminal_forest(&[run]))
        );
        assert!(text_of(&opened).contains("land the session shell"));
    }

    /// The fold control on line `at` of a page, whole.
    fn fold_on(html: &str, at: usize) -> &str {
        let line = line_html(html, at);
        let fold = &line[line.find("<a class=\"fold").expect("the line folds")..];
        &fold[..fold.find("</a>").expect("the fold ends") + "</a>".len()]
    }

    /// The fold is a control of its own beside what its line hangs under,
    /// rather than a link around it, so the rails and the rest of the row
    /// are not a fold target. It says whether its fold is open, and is
    /// named for what it folds.
    #[test]
    fn a_fold_is_a_control_of_its_own_named_for_what_it_folds() {
        let html = page_at("/").html;
        let shut = lines_titled(&html, "apply the two niri settings")[0];
        assert!(
            fold_on(&html, shut).ends_with(
                " aria-expanded=\"false\" aria-label=\"Unfold apply the two niri settings\"></a>"
            ),
            "{}",
            fold_on(&html, shut)
        );

        let opened = page_at(&fold_link(&html, shut)).html;
        assert!(
            fold_on(&opened, shut).ends_with(
                " aria-expanded=\"true\" aria-label=\"Fold apply the two niri settings\"></a>"
            ),
            "{}",
            fold_on(&opened, shut)
        );
    }

    /// Every fold on the trees and on the ready list is the same control,
    /// drawing nothing of the line it folds.
    #[test]
    fn every_fold_in_the_trees_and_the_ready_list_is_a_control_of_its_own() {
        for target in ["/", "/?ready=1"] {
            let html = page_at(target).html;
            let folding: Vec<usize> = (0..lines_html(&html).len())
                .filter(|&at| line_html(&html, at).contains("<a class=\"fold"))
                .collect();
            assert!(!folding.is_empty(), "{target} draws a fold");
            for at in folding {
                let fold = fold_on(&html, at);
                let named =
                    fold.contains(" aria-label=\"Fold ") || fold.contains(" aria-label=\"Unfold ");
                let says = fold.contains(" aria-expanded=\"");
                assert!(
                    named && says && fold.ends_with("\"></a>"),
                    "{target}: {fold}"
                );
            }
        }
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
        let head = inside(&answer.html, "head");
        assert!(
            head.contains("<h2>apply the two niri settings</h2>"),
            "{head}"
        );
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
        assert!(text_of(&answer.html).contains("This bead is not in any tree smt-zzz"));
        assert!(
            answer.html.contains(&not_in_any_tree(&key("smt-zzz"))),
            "{}",
            answer.html
        );
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

    /// Under its head, the bead view on the page is the terminal's, row for
    /// row, with every bead it names drawn as a reference to that bead. The
    /// row saying what the bead is blocked by is the one the page leaves
    /// out, because the head's band says it.
    #[test]
    fn the_bead_view_draws_the_terminals_rows_with_every_bead_named_by_a_reference() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        let view = &html[html.find("<div class=\"page\">").expect("the bead view")..];
        let head = view.find("<div class=\"head\">").expect("the head");
        let under_head = &view[element_end(view, head)..view.find("</aside>").unwrap()];
        let page_rows: Vec<String> = text_of(&in_the_terminals_order(under_head))
            .lines()
            .skip(1)
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
            .enumerate()
            .skip(said.head)
            .filter(|(at, _)| *at != blocked_by)
            .map(|(_, row)| {
                words(
                    &row.iter()
                        .map(|span| span.content.as_ref())
                        .collect::<String>(),
                )
            })
            .collect();

        assert_eq!(page_rows, terminal_rows);
        assert!(terminal_rows
            .iter()
            .any(|row| row == "→ ○ smt-4kd3p.1 wire the niri theme include"));
    }

    /// The head of a bead's page says its status in words and its id, then
    /// its title as the page's heading.
    #[test]
    fn a_bead_page_heads_with_its_status_in_words_and_its_id_over_its_title() {
        let html = page_at(&address(&key("smt-4kd3p"))).html;
        let head = inside(&html, "head");
        assert_eq!(parts(head).0, ["p.state", "h2"], "{head}");
        assert_eq!(
            words(&text_of(inside(head, "state"))),
            "◐ In progress smt-4kd3p"
        );
        assert!(head.contains("<h2>lift the ground station</h2>"), "{head}");
    }

    /// The status pill takes its status's colour from the stylesheet, and
    /// its mark is hidden from a reader who hears the page, because the
    /// words beside it say the status.
    #[test]
    fn a_bead_pages_status_pill_is_classed_by_its_status_and_says_it_once() {
        let html = page_at(&address(&key("smt-4kd3p"))).html;
        let state = inside(inside(&html, "head"), "state");
        assert_eq!(
            parts(state).0,
            ["span.status in-progress", "span.id"],
            "{state}"
        );
        let pill = inside(state, "status in-progress");
        assert!(!pill.contains("style="), "{pill}");
        assert!(!pill.contains("role=\"img\""), "{pill}");
        assert!(pill.contains("aria-hidden=\"true\""), "{pill}");
    }

    /// A bead's title is set as the page's heading in the display face at
    /// 22 pixels, at every width, keeping the spaces its author typed as
    /// the terminal does.
    #[test]
    fn a_bead_pages_title_is_set_large_in_the_display_face() {
        assert!(STYLE.contains(
            ".page .head h2{margin:0;font:600 22px/1.25 var(--display);white-space:pre-wrap;"
        ));
        assert!(!STYLE.contains(".page .head .title"));
    }

    /// A blocked bead's head lists what it waits on in a band under its
    /// title, each blocker drawn as a reference, and the rows under the head
    /// no longer say it.
    #[test]
    fn a_blocked_beads_head_says_what_it_waits_on_under_its_title() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        let head = inside(&html, "head");
        assert_eq!(parts(head).0, ["p.state", "h2", "div.waits"], "{head}");
        assert_eq!(
            waiting_on(ground()),
            [
                "Waiting on 1 bead",
                "○ wire the niri theme include smt-4kd3p.1"
            ]
        );
        assert!(!text_of(&html).contains(phrase::blocked_by_ahead()));
    }

    /// Ids are per tracker, so a bead can depend on two beads of one id. The
    /// one the band names is the one that blocks it.
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
        assert_eq!(
            waiting_on(ground),
            [
                "Waiting on 1 bead",
                "○ wire the niri theme include smt-4kd3p.1"
            ]
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

    /// What the band a blocked bead's head draws under its title says above
    /// its blockers, then the words of each blocker's reference.
    fn waiting_on(snapshot: Snapshot) -> Vec<String> {
        let html = page_of(snapshot, &address(&key("smt-4kd3p.6")));
        let (references, rest) = references_and_the_rest(inside(&html, "waits"));
        std::iter::once(words(&text_of(&rest)))
            .chain(
                references
                    .iter()
                    .map(|reference| words(&text_of(reference))),
            )
            .collect()
    }

    #[test]
    fn a_blocker_that_is_no_dependency_of_the_bead_is_still_named_by_its_reference() {
        let snapshot = with_the_bead_changed(|bead| {
            bead.blocked_by.push("smt-4kd3p.8".to_string());
        });
        assert_eq!(
            waiting_on(snapshot),
            [
                "Waiting on 2 beads",
                "○ wire the niri theme include smt-4kd3p.1",
                "○ read the wedge log smt-4kd3p.8"
            ]
        );
    }

    /// A blocker a tracker names twice is one bead, waited on once.
    #[test]
    fn a_blocker_named_twice_is_waited_on_once() {
        let snapshot = with_the_bead_changed(|bead| {
            bead.blocked_by.push("smt-4kd3p.1".to_string());
        });
        assert_eq!(
            waiting_on(snapshot),
            [
                "Waiting on 1 bead",
                "○ wire the niri theme include smt-4kd3p.1"
            ]
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
            waiting_on(snapshot),
            [
                "Waiting on 2 beads",
                "○ wire the niri theme include smt-4kd3p.1",
                "○ read the wedge log smt-4kd3p.8"
            ]
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
            waiting_on(ground),
            [
                "Waiting on 2 beads",
                "○ wire the niri theme include smt-4kd3p.1",
                "○ read the wedge log smt-4kd3p.8"
            ]
        );
        let band = inside(&html, "waits");
        assert!(band.contains("href=\"/faraway/smt-4kd3p.8"), "{band}");
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
            waiting_on(ground),
            [
                "Waiting on 2 beads",
                "○ wire the niri theme include smt-4kd3p.1",
                "○ a bead of another tracker smt-4kd3p.8"
            ]
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
            waiting_on(ground),
            [
                "Waiting on 3 beads",
                "○ wire the niri theme include smt-4kd3p.1",
                "○ a bead of one tracker smt-4kd3p.8",
                "○ a bead of another smt-4kd3p.8"
            ]
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
        assert!(answer.html.contains(&not_in_any_tree(&key("smt-4kd3p"))));
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
    /// the terminal says ready, with no band of what it waits on.
    #[test]
    fn a_bead_said_to_be_ready_and_blocked_is_said_to_be_ready_as_the_terminal_says() {
        let snapshot = with_the_bead_changed(|bead| bead.ready = true);
        let html = page_of(snapshot, &address(&key("smt-4kd3p.6")));
        let aside = &html[html.find("<aside>").expect("the bead view")..];
        assert!(!aside.contains("class=\"waits\""), "{aside}");
        assert!(text_of(aside).lines().any(|line| line.trim() == "ready"));
    }

    /// Every bead on every page is named by the one component: no bead's id
    /// appears outside a reference, and every reference holds a bead's title.
    /// A bead's page heads with the bead itself, which is no reference.
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
            let html = match html.find("<p class=\"state\">") {
                Some(state) => format!("{}{}", &html[..state], &html[element_end(&html, state)..]),
                None => html,
            };
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
        for at in beads {
            let line = line_html(&html, at);
            let (shape, between) = parts(line);
            assert_eq!(shape, ["span.rails", "a.bead", "span.state"], "{line}");
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

    /// A status's colour is drawn on the bead's status mark and nowhere
    /// else: not on an id, a count, a rail or a word beside it. Colour alone
    /// then says the status, because nothing else on the page is that colour.
    #[test]
    fn only_a_status_mark_carries_a_status_colour() {
        let mut snapshot = ground();
        let tree = std::sync::Arc::make_mut(&mut snapshot.trees[0]);
        tree.beads[6].status = Status::Blocked;
        tree.beads[8].status = Status::Deferred;
        tree.beads[1].status = Status::Hooked;
        snapshot.collected = snapshot.trees.clone();
        let targets = ["/", "/?ready=1"].map(String::from).into_iter().chain(
            [
                "smt-4kd3p",
                "smt-4kd3p.1",
                "smt-4kd3p.2",
                "smt-4kd3p.6",
                "smt-4kd3p.8",
            ]
            .map(|id| address(&key(id))),
        );
        let mut coloured = Vec::new();
        for target in targets {
            let html = page_of(snapshot.clone(), &target);
            let body = &html[html.find("<body").expect("the page has a body")..];
            coloured.extend(
                body.split('<')
                    .map(|tag| &tag[..tag.find('>').unwrap_or(tag.len())])
                    .filter(|tag| {
                        let status = tag.contains("--status-") || tag.contains("--accent");
                        status && !tag.contains("role=\"img\"")
                    })
                    .map(|tag| format!("{target}: <{tag}>")),
            );
        }
        assert_eq!(coloured, Vec::<String>::new());
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
            ["span.rails", "a.bead", "span.more", "span.state"],
            "{line}"
        );
        let more = inside(line, "more");
        assert!(
            more.contains("href=\"https://example.invalid/dunwich/12\""),
            "{more}"
        );
        assert_eq!(words(&text_of(more)), "⇢ #12");
        assert_eq!(parts(more).0, ["a.pill"], "{more}");
    }

    /// The ground beside a second project, whose one tree is a bead ready
    /// to start.
    fn beside_dunwich() -> Snapshot {
        let mut ground = ground();
        let mut reflector = node("dun-7", "re-silver the reflector", Status::Open);
        reflector.project = "dunwich".to_string();
        reflector.ready = true;
        let dunwich = Tree {
            beads: vec![reflector],
            children: vec![vec![]],
            counts: counts(0, 1, 0, 0),
            ..tree(
                "dunwich",
                "dun-7",
                "re-silver the reflector",
                counts(0, 0, 0, 0),
            )
        };
        let mut trees: Vec<Tree> = ground.trees.iter().map(|tree| (**tree).clone()).collect();
        trees.push(dunwich);
        let mut both = snapshot(trees, Vec::new(), ProviderState::Absent);
        both.views = std::mem::take(&mut ground.views);
        both
    }

    /// Each reference among the lines of `html` that names a project, as
    /// its id and the project it names.
    fn projects_named(html: &str) -> Vec<(String, String)> {
        bead_lines(html)
            .into_iter()
            .filter(|line| line.contains("class=\"project\""))
            .map(|line| {
                let reference = inside(line, "bead");
                let (shape, _) = parts(reference);
                assert_eq!(
                    shape[1..],
                    ["span.title", "span.id", "span.project"],
                    "{line}"
                );
                (
                    text_of(inside(reference, "id")),
                    text_of(inside(reference, "project")),
                )
            })
            .collect()
    }

    /// A list of beads from more than one project has no heading over each
    /// project's rows, so each of its rows names its project in a quiet tag
    /// after its id.
    #[test]
    fn a_row_of_a_list_across_projects_names_its_project_after_its_id() {
        let ready = Asked::default().listing(listed(None, true));
        let html = page_of(beside_dunwich(), &ready);
        let named = projects_named(&html);
        assert_eq!(named.len(), bead_lines(&html).len(), "{html}");
        assert!(
            named.contains(&("dun-7".to_string(), "dunwich".to_string())),
            "{named:?}"
        );
        assert!(
            named.contains(&("smt-4kd3p.4".to_string(), PROJECT.to_string())),
            "{named:?}"
        );

        let view = Asked::default().listing(listed(Some(VIEWS[1].0.to_string()), false));
        let named = projects_named(&page_of(beside_dunwich(), &view));
        assert!(!named.is_empty());
        assert!(
            named.iter().all(|(_, project)| project == PROJECT),
            "{named:?}"
        );
    }

    /// Where the page says which project a row is in some other way, no row
    /// names it: under the trees each project has its heading, a list kept
    /// to one project is headed by it, and with one project there is no
    /// other the row could be in.
    #[test]
    fn a_row_names_no_project_where_the_page_already_says_it() {
        let ready = Asked::default().listing(listed(None, true));
        let left = Asked::default().listing(Some(List::LeftToDo {
            project: PROJECT.to_string(),
        }));
        for (snapshot, target) in [
            (beside_dunwich(), "/".to_string()),
            (beside_dunwich(), left),
            (ground(), ready),
        ] {
            let html = page_of(snapshot, &target);
            assert!(!bead_lines(&html).is_empty(), "{target}");
            assert!(!html.contains("class=\"project\""), "{target}: {html}");
        }
    }

    /// A row's project is quiet, and makes room beside an open bead on a
    /// desktop as a flag's words do. A phone draws it on the row's second
    /// line, in a column of its own between the id and the row's other
    /// cells, and breaks a long name there rather than push them aside.
    #[test]
    fn a_rows_project_is_drawn_after_its_id_and_gives_way_to_an_open_bead() {
        assert!(STYLE.contains(".bead .project{padding:0 8px;border:1px solid var(--line);"));
        assert!(
            rules_under("(width>=1100px)").contains("main.open .line .bead>.project{display:none}")
        );
        let phone = rules_under("(max-width:720px)");
        for rule in [
            "grid-template-columns:auto 1.6em repeat(4,auto) minmax(0,1fr)",
            ".line>.bead>.project{grid-area:2/3;justify-self:start;margin-left:8px;overflow-wrap:anywhere}",
            ".line>.more{grid-column:5;",
            ".line>.bead~.state{grid-column:6;",
        ] {
            assert!(phone.contains(rule), "{rule}");
        }
    }

    /// The ground beside dunwich, with dunwich read an hour before the page
    /// is drawn, so it is the older of the two reads.
    fn dunwich_read_an_hour_ago() -> (Snapshot, DateTime<Utc>) {
        let an_hour_ago = drawn_at() - chrono::TimeDelta::hours(1);
        let mut both = beside_dunwich();
        both.read_at.insert("dunwich".to_string(), an_hour_ago);
        (both, an_hour_ago)
    }

    /// What the one line that the projects fold into says.
    fn folded(html: &str) -> &str {
        let from = html
            .find("<details class=\"projects\"><summary>")
            .expect("the projects are folded");
        &html[from..from + html[from..].find("</summary>").expect("the line ends")]
    }

    /// A list across projects has no rows under each project's heading, so
    /// the headings fold into one line above the list, which opens to them.
    /// The line counts the projects and says when the oldest of them was
    /// read, once that is old enough to show, as a heading does.
    #[test]
    fn a_list_across_projects_folds_their_headings_into_one_line_that_opens_to_them() {
        let (both, an_hour_ago) = dunwich_read_an_hour_ago();
        let ready = Asked::default().listing(listed(None, true));
        let view = Asked::default().listing(listed(Some(VIEWS[1].0.to_string()), false));
        for target in [ready, view] {
            let html = page_of(both.clone(), &target);
            let headings = inside(&html, "projects");
            assert_eq!(headings.matches("<div class=\"line heading\"").count(), 2);
            assert_eq!(html.matches("<div class=\"line heading\"").count(), 2);
            assert!(headings.contains("<h2>dunwich</h2>"), "{headings}");
            assert!(
                headings.contains(&format!("<h2>{PROJECT}</h2>")),
                "{headings}"
            );

            let line = folded(&html);
            assert!(
                words(&text_of(line)).starts_with("2 projects read "),
                "{line}"
            );
            let read = format!(
                "<span data-shown-once-older-than=\"120\"> read <time datetime=\"{}\"",
                an_hour_ago.to_rfc3339()
            );
            assert!(line.contains(&read), "{line}");
            assert!(!line.contains("class=\"flag\""), "{line}");
        }
    }

    /// Only a project whose read went badly is named on the folded line, in
    /// the colour of a flag with the mark its heading draws. The rest are
    /// counted, and the read time is the oldest of theirs.
    #[test]
    fn the_folded_line_names_only_a_project_whose_read_went_badly() {
        let (both, _) = dunwich_read_an_hour_ago();
        let fresh = both.read_at[PROJECT];
        let stuck = Drawn {
            collecting: vec![Awaited {
                wanted: crate::app::Wanted::Project("dunwich".to_string()),
                asked_at: drawn_at() - chrono::TimeDelta::seconds(60),
                patience: chrono::TimeDelta::seconds(30),
            }],
            ..drawing()
        };
        let ready = Asked::default().listing(listed(None, true));
        let asked = Asked::of(&ready).expect("the ready list");
        let html = page(both, &Layout::default(), &asked, stuck).html;

        let line = folded(&html);
        assert!(
            words(&text_of(line)).starts_with("1 project read "),
            "{line}"
        );
        assert!(line.contains(&fresh.to_rfc3339()), "{line}");
        let unanswered = phrase::mark(
            crate::view::Freshness {
                mark: crate::view::Mark::Unanswered,
                read_at: None,
            },
            drawn_at(),
        );
        assert!(
            line.contains(&format!("<span class=\"flag\">{unanswered} dunwich</span>")),
            "{line}"
        );
        assert!(!text_of(line).contains(PROJECT), "{line}");
    }

    /// The trees keep a heading over each project's own trees, a list kept
    /// to one project is headed by it, and with one project there is no
    /// other to fold away.
    #[test]
    fn the_trees_and_a_list_of_one_project_keep_their_headings_unfolded() {
        let ready = Asked::default().listing(listed(None, true));
        let left = Asked::default().listing(Some(List::LeftToDo {
            project: PROJECT.to_string(),
        }));
        for (snapshot, target) in [
            (beside_dunwich(), "/".to_string()),
            (beside_dunwich(), left),
            (ground(), ready),
        ] {
            let html = page_of(snapshot, &target);
            assert!(html.contains("<div class=\"line heading\""), "{target}");
            assert!(!html.contains("class=\"projects\""), "{target}: {html}");
        }
    }

    /// The folded line is drawn as a control, with a chevron at its end
    /// that turns over once the line is open.
    #[test]
    fn the_folded_line_ends_in_a_chevron_that_turns_when_it_opens() {
        for rule in [
            ".projects>summary{list-style:none;cursor:pointer;",
            ".projects>summary::-webkit-details-marker{display:none}",
            ".projects>summary::after{content:\"\";",
            ".projects[open]>summary::after{transform:rotate(180deg)}",
        ] {
            assert!(STYLE.contains(rule), "{rule}");
        }
    }

    /// The beads of the ground's lines on the trees, in order.
    fn bead_lines(html: &str) -> Vec<&str> {
        (0..lines_html(html).len())
            .map(|at| line_html(html, at))
            .filter(|line| line.contains("class=\"bead\""))
            .collect()
    }

    /// A bead's progress is the last thing on its row, a bar filled as far
    /// as the bead is through beside the fraction. A row with no progress
    /// keeps the column empty, so what comes before it lines up with the
    /// rows that have one.
    #[test]
    fn a_bead_rows_progress_is_drawn_last_as_a_bar() {
        let html = page_at("/").html;
        let rows = bead_lines(&html);
        for row in &rows {
            let (state, _) = parts(inside(row, "state"));
            assert_eq!(state.last().map(String::as_str), Some("span.prog"), "{row}");
        }
        let root = inside(rows[0], "prog");
        assert_eq!(words(&text_of(root)), "3/9", "{root}");
        assert!(root.contains("<span style=\"width:33%\"></span>"), "{root}");
        assert_eq!(text_of(inside(rows[0], "state")).trim(), "3/9");
        let leaf = rows
            .iter()
            .find(|row| text_of(row).contains("smt-4kd3p.4"))
            .expect("the leaf has a row");
        assert!(
            leaf.ends_with("<span class=\"prog\"></span></span>"),
            "{leaf}"
        );
    }

    /// Progress is drawn last on the row whichever block or place the
    /// layout gives it, so a flag drawn after it in the terminal comes
    /// before it on the page.
    #[test]
    fn a_rows_progress_is_drawn_last_wherever_the_layout_puts_it() {
        let checked = with_the_bead_changed(|node| {
            node.anomalies = vec![Anomaly::StaleClaim { days: 58 }];
        });
        let layout = Layout {
            title: vec![Cell::Title, Cell::Progress, Cell::Badges],
            ..Layout::default()
        };
        let asked = Asked::of(&address(&key("smt-4kd3p.6"))).expect("the bead's page");
        let html = page(checked, &layout, &asked, drawing()).html;
        let row = bead_lines(&html)
            .into_iter()
            .find(|row| text_of(row).contains("apply the two niri settings"))
            .expect("the bead has a row");
        let (state, _) = parts(inside(row, "state"));
        assert_eq!(state, ["span.flag", "span.prog"], "{row}");
        assert!(
            words(&text_of(inside(row, "state"))).ends_with(" 0/3"),
            "{row}"
        );
        assert!(!row.contains("class=\"more\""), "{row}");
    }

    /// A layout that draws no progress gives no row a column for it.
    #[test]
    fn a_layout_without_progress_draws_no_progress_column() {
        let layout = Layout {
            state: vec![Cell::Agent, Cell::Anomalies],
            ..Layout::default()
        };
        let asked = Asked::of("/").expect("the forest");
        let html = page(ground(), &layout, &asked, drawing()).html;
        for row in bead_lines(&html) {
            assert!(!row.contains("class=\"prog\""), "{row}");
        }
    }

    /// On a desktop a row fits the trees' width. Its title gives way and
    /// ends in an ellipsis, before its id, but keeps a few characters, and
    /// the reference keeps room for them however wide the cells beside it.
    /// On a row too crowded for its id the reference cuts it off at its own
    /// edge rather than run under those cells, and badges too wide for the
    /// row end in an ellipsis rather than push the bar out of its column.
    /// Everything after the reference sits together at the right, with the
    /// progress column a fixed width at the end and its bar at the column's
    /// start, so the bars line up whatever their fractions say. A project's
    /// heading is not a bead's row, and its bar keeps its own width.
    #[test]
    fn a_desktop_row_ellipsises_its_title_before_the_columns_at_its_right() {
        assert!(!STYLE.contains("max-content"), "{STYLE}");
        let desktop = rules_under("(width>720px)");
        for rule in [
            ".line>.bead{display:flex;align-items:center;gap:10px;min-width:12ch;overflow:hidden;margin-right:auto}",
            ".line>.bead>*{flex:none}",
            ".line>.bead>.title{flex:0 1 auto;min-width:8ch;overflow:hidden;text-overflow:ellipsis}",
            ".line>.more{display:flex;align-items:center;gap:8px;min-width:0;padding-left:2ch;overflow:hidden}",
            ".line>.more>.pill{min-width:0;overflow:hidden;text-overflow:ellipsis}",
            ".line>.bead~.state{margin-left:0}",
            ".line>.bead~.state>.prog{width:104px}",
        ] {
            assert!(desktop.contains(rule), "{desktop}");
        }
    }

    /// A phone row's progress is its fraction alone, with no bar to make the
    /// row wider than the screen.
    #[test]
    fn a_phone_row_draws_its_progress_without_a_bar() {
        let phone = rules_under("(max-width:720px)");
        assert!(
            phone.contains(".line>.bead~.state>.prog>.bar{display:none}"),
            "{phone}"
        );
    }

    /// A pill leaves wrapping to its row, so a long badge wraps on a phone
    /// rather than push the row wider than the screen.
    #[test]
    fn a_pill_wraps_where_its_row_does() {
        let pill = STYLE
            .split(".pill{")
            .nth(1)
            .and_then(|rest| rest.split('}').next());
        assert!(
            pill.is_some_and(|rule| !rule.contains("white-space")),
            "{STYLE}"
        );
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

    /// The elements drawn as rails on line `at`, each as its tag and its
    /// class: the levels inside what the line hangs under, without its fold.
    fn rails(html: &str, at: usize) -> Vec<String> {
        let line = line_html(html, at);
        let hung = &line[..element_end(line, 0)];
        let content = hung.find('>').expect("the tag ends") + 1;
        parts(&hung[content..hung.rfind('<').expect("a closing tag")])
            .0
            .into_iter()
            .filter(|part| !part.starts_with("a.fold"))
            .collect()
    }

    /// Each level a line hangs under is a rail, except the column a
    /// project's roots hang in: a trunk where a line below still hangs at
    /// that level, a gap where none does, and the line's own elbow, marked
    /// where it is the last there and where it hangs on a blocking edge. On
    /// a line deeper than a phone indents, the levels between its outermost
    /// four and its elbow are marked as past a phone's indent.
    #[test]
    fn each_level_a_line_hangs_under_is_a_rail_and_a_phone_indents_four() {
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
                rails("soak it in oil", &["i", "i", "b", "b.deep", "u.last"]),
                rails(
                    "wait a night",
                    &["i", "i", "b", "b.deep", "b.deep", "u.last"]
                ),
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

    /// A fold's chevron is masked on a box of its own inside the fold, never
    /// on the fold itself, whose focus ring a mask would clip.
    #[test]
    fn no_fold_link_is_masked_itself() {
        let masked: Vec<&str> = STYLE
            .split('}')
            .filter(|rule| rule.contains("mask:"))
            .filter_map(|rule| rule.split('{').rev().nth(1))
            .flat_map(|selectors| selectors.split(','))
            .filter(|selector| selector.contains("fold"))
            .collect();
        assert!(!masked.is_empty(), "{STYLE}");
        for selector in masked {
            assert!(selector.ends_with("::before"), "{selector}");
        }
    }

    /// Whatever a line hangs under ends in a box as wide as the fold, folding
    /// or not, sized at each width by selectors as specific as the other
    /// width's. So a row that folds starts its title where a row beside it
    /// that does not fold starts its own.
    #[test]
    fn every_prefix_ends_in_a_box_as_wide_as_the_fold_at_each_width() {
        let phone = STYLE
            .find("@media (max-width:720px){")
            .expect("a phone's rules");
        for (rules, width) in [
            (&STYLE[..phone], "width:28px"),
            (&STYLE[phone..], "width:32px"),
        ] {
            let sized: Vec<&str> = rules
                .split('}')
                .filter(|rule| rule.contains(width))
                .collect();
            for selector in [
                ".line>:has(+.bead):not(:has(>.fold))::after",
                ".line .rails:not(:has(>.fold))::after",
                ".line a.fold{",
            ] {
                assert!(
                    sized.iter().any(|rule| rule.contains(selector)),
                    "{width}: {selector}: {sized:#?}"
                );
            }
        }
    }

    /// The page draws a fold's chevron itself, so a fold says whether it is
    /// shut.
    #[test]
    fn a_shut_fold_says_so_and_an_open_one_does_not() {
        let html = page_at("/").html;
        let folds: Vec<(String, bool)> = (0..lines_html(&html).len())
            .filter(|&at| line_html(&html, at).contains("class=\"bead"))
            .filter(|&at| line_html(&html, at).contains("<a class=\"fold"))
            .map(|at| {
                let line = line_html(&html, at);
                (
                    words(&text_of(inside(line, "title"))),
                    line.contains("<a class=\"fold shut"),
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
    /// own colours, and only code is set in the code face.
    #[test]
    fn the_page_is_set_in_a_proportional_face_in_its_own_colours() {
        let html = page_at(&address(&key("smt-4kd3p.6"))).html;
        assert!(html.contains(&palette::page_colours()), "{html}");
        assert!(
            STYLE.contains("body{margin:0;background:var(--surface);color:var(--fg);font:14px/1.45 var(--body)}"),
            "{STYLE}"
        );
        assert_eq!(
            STYLE
                .match_indices("var(--code)")
                .map(|(at, _)| &STYLE[STYLE[..at].rfind('}').map_or(0, |end| end + 1)..at])
                .collect::<Vec<_>>(),
            [".code{font-family:"],
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
    fn where_the_config_names_no_view_the_menu_offers_the_trees_and_the_beads_to_check() {
        let mut bare = ground();
        bare.views.clear();
        let asked = Asked::of("/").expect("the forest");
        let html = page(bare, &Layout::default(), &asked, drawing()).html;
        let menu = inside(&html, "lists");
        assert_eq!(menu.matches("<a ").count(), 2, "{menu}");
        assert!(!menu.contains(phrase::views()), "{menu}");
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

    /// Under the band saying one tree is shown, the project's heading counts
    /// that tree rather than the whole project.
    #[test]
    fn a_heading_over_one_tree_counts_that_tree() {
        let rooted = page_at(&link_saying(
            &page_at(&address(&key("smt-4kd3p.6"))).html,
            phrase::only_this_tree(),
        ))
        .html;

        assert_eq!(
            page_lines(&rooted)[0],
            format!("▾ summit-works {READ} 3 left of 3")
        );
    }

    /// A project's heading is a tinted band that stays at the top of the
    /// window while its trees scroll. A box between it and the window that
    /// scrolls on its own would hold it there instead, and it would never
    /// stick.
    #[test]
    fn a_projects_heading_is_a_tinted_band_that_sticks_to_the_top_of_the_window() {
        let rule = |selector: &str| {
            let at = STYLE.find(&format!("{selector}{{")).expect(selector);
            &STYLE[at..at + STYLE[at..].find('}').expect("a rule ends")]
        };
        let heading = rule(".line.heading");
        for said in ["position:sticky", "top:0", "background:var(--sunk)"] {
            assert!(heading.contains(said), "{heading}");
        }
        let forest = rule(".forest");
        assert!(
            !forest.contains("auto") && !forest.contains("scroll"),
            "{forest}"
        );
    }

    /// A project's heading and the lines under it are one section, which is
    /// what holds a stuck heading to its own project's lines.
    #[test]
    fn a_projects_heading_and_the_lines_under_it_are_one_section() {
        let html = page_at("/").html;
        let opened = html.find("<section>").expect("a section");
        let section = &html[opened..html.find("</section>").expect("the section ends")];

        assert!(
            section.starts_with("<section><div class=\"line heading\""),
            "{section}"
        );
        assert_eq!(lines_html(section).len(), lines_html(&html).len());
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
    /// page shows only this tree, then offers every tree back. The root's
    /// row under it gives its title, so the band does not repeat it.
    #[test]
    fn the_band_over_one_tree_says_showing_only_this_tree_then_show_all() {
        let bead = key("smt-4kd3p.6");
        let rooted = page_at(&link_saying(
            &page_at(&address(&bead)).html,
            phrase::only_this_tree(),
        ))
        .html;
        let band = &rooted[rooted.find("<p class=\"rooted\">").expect("the band")..];
        let band = text_of(&band[..band.find("</p>").expect("the band ends")]);
        let said = [phrase::showing_only(), phrase::this_tree(), phrase::show_all()].map(|said| {
            band.find(said)
                .unwrap_or_else(|| panic!("{said:?} in {band:?}"))
        });
        assert!(said.is_sorted(), "{band:?}");
        let ground = ground();
        let title = &ground.node(&bead).expect("the bead is held").title;
        assert!(!band.contains(title.as_str()), "{band:?}");
        assert_eq!(
            [phrase::showing_only(), phrase::this_tree(), phrase::show_all()],
            ["Showing only", "this tree", "Show all"]
        );
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

    /// The band is one line at any width, and a bead in no tree is warned
    /// of on amber, the ground every warning uses, never the band's teal.
    #[test]
    fn the_band_keeps_to_one_line_and_the_warning_is_amber() {
        assert!(
            STYLE.contains(".rooted{flex-wrap:nowrap;white-space:nowrap;"),
            "{STYLE}"
        );
        assert!(
            STYLE.contains(".missing{background:var(--warn-soft);color:var(--warn)}"),
            "{STYLE}"
        );
    }

    /// An address rooted at a bead the forest no longer draws warns over
    /// every tree, rather than on the band that says one tree is shown, and
    /// offers them without the root.
    #[test]
    fn a_root_the_forest_no_longer_draws_is_warned_of() {
        let gone = key("smt-4kd3p.99");
        let html = page_at("/?root=summit-works%3Asmt-4kd3p%2Fsummit-works%3Asmt-4kd3p.99").html;
        assert!(!html.contains("<p class=\"rooted\">"), "{html}");
        assert!(html.contains(&not_in_any_tree(&gone)), "{html}");
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
        let ground = with_the_bead_changed(|bead| {
            bead.depends_on.push(Related {
                id: "smt-4kd3p.7".to_string(),
                project: None,
                edge: Edge::Other("related".to_string()),
                status: Some(Status::Open),
                title: Some("confirm the wedges are gone".to_string()),
            });
        });
        let rooted = page_of(
            ground.clone(),
            &link_saying(
                &page_of(ground, &address(&key("smt-4kd3p.6"))),
                phrase::only_this_tree(),
            ),
        );
        let named = named_by_the_bead_view(&rooted);
        assert!(
            named["smt-4kd3p"].root.is_none(),
            "the parent is outside the tree"
        );
        assert!(
            named["smt-4kd3p.7"].root.is_some(),
            "a bead under the root is inside it"
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
            named["smt-4kd3p.1"].list.is_some(),
            "the view lists its blocker"
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
