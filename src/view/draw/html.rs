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
use crate::view::address::{fold_name, Asked, SHOWN};
use crate::view::fitted::{indent, Block, Link};
use crate::view::forest::Forest;
use crate::view::lines::{Content, Line, Waiting};
use crate::view::row::{Layout, WARNING};
use crate::view::{palette, phrase, show};

use super::bead::referenced_line;
use super::foot::notices;
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

/// When a page is drawn, the offset from UTC it says the time of day at, and
/// the reads out as it is drawn.
#[derive(Debug, Clone)]
pub struct Drawn {
    pub at: DateTime<Utc>,
    pub offset: FixedOffset,
    pub collecting: Vec<Awaited>,
}

/// The page `asked` names, drawn from `snapshot` under `layout`.
///
/// A page is drawn once and may be left open, so each project's line says
/// the time of day it was read at rather than how long ago, which the
/// terminal says because it redraws itself as the age moves.
pub fn page(snapshot: Snapshot, layout: &Layout, asked: &Asked, drawn: Drawn) -> Answer {
    let shown = asked.forest(snapshot, layout);
    let forest = &shown.forest;
    let node = show::selected(forest).filter(|_| shown.bead == Some(true));
    let asked = &shown.asked;
    let drawing = Drawing {
        forest,
        layout,
        asked,
        selected: node.map(|_| forest.selected_line()),
    };

    let mut html = String::new();
    html.push_str(&format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title><style>{STYLE}</style></head><body>",
        escaped(node.map_or("bdi", |node| &node.title))
    ));
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
            escaped(&asked.closing()),
            escaped(phrase::back_to_the_forest())
        ));
        match node {
            Some(node) => drawing.bead_view(&mut html, node),
            None => html.push_str(&format!(
                "<p class=\"missing\">{}</p>",
                escaped(&format!("{WARNING} {}", phrase::not_drawn(key)))
            )),
        }
        html.push_str("</aside>");
    }
    html.push_str("</main>");
    drawing.foot(&mut html);
    html.push_str("</body></html>\n");
    Answer {
        found: asked.bead.is_none() || node.is_some(),
        html,
    }
}

/// A page with nothing on it but one sentence and a link to the forest, for
/// an address that has no forest to draw.
pub fn saying(said: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>bdi</title><style>{STYLE}</style></head><body><footer><p>{}</p>\
         <p><a class=\"close\" href=\"/\">{}</a></p></footer></body></html>\n",
        escaped(said),
        escaped(phrase::back_to_the_forest())
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
    fn forest_lines(&self, html: &mut String, drawn: Drawn) {
        let lines = self.forest.lines();
        let reads = Reads::new(
            &self.forest.snapshot().read_at,
            &drawn.collecting,
            &[],
            drawn.at,
        )
        .dated_at(drawn.offset);
        for (at, line) in lines.viewport(0, lines.len()) {
            self.forest_line(html, at, line, &reads);
        }
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
        let selected = self.selected == Some(at);
        let anchor = fold_name(line).map_or_else(String::new, |name| format!(" id=\"{name}\""));
        html.push_str(&format!(
            "<div class=\"line{}\"{anchor}{}>",
            if selected { " shown" } else { "" },
            styled(palette::css(whole.under))
        ));
        if selected {
            html.push_str(&format!("<a id=\"{SHOWN}\"></a>"));
        }
        html.push_str("<span class=\"left\">");
        let (mut identity, mut from) = (whole.identity, 0);
        if let Some((prefix, rest)) = identity.split_first() {
            if prefix.content == line.prefix {
                self.prefix(html, line, prefix);
                (identity, from) = (rest, 1);
            }
        }
        if let Some(reference) = reference {
            let href = line
                .place
                .as_ref()
                .map(|place| self.asked.opening_at(place));
            self.reference(html, reference, href);
            if !identity.is_empty() {
                html.push_str(&indent());
            }
        }
        spans(html, identity, Block::Identity, whole.links, from);
        if !whole.title.is_empty() {
            html.push_str(&indent());
            spans(html, whole.title, Block::Title, whole.links, 0);
        }
        html.push_str("</span><span class=\"state\">");
        spans(html, whole.state, Block::State, whole.links, 0);
        html.push_str("</span></div>\n");
    }

    /// The box-drawing a line hangs under, which is the link that toggles the
    /// line's fold where it has one.
    fn prefix(&self, html: &mut String, line: &Line, prefix: &Span<'static>) {
        let Some(href) = self.asked.toggling(line) else {
            spans(html, std::slice::from_ref(prefix), Block::Identity, &[], 0);
            return;
        };
        let said = if line.folded == Some(true) {
            phrase::shut_the_fold()
        } else {
            phrase::open_the_fold()
        };
        html.push_str(&format!(
            "<a class=\"fold\" href=\"{}\" aria-label=\"{}\"{}>{}</a>",
            escaped(&href),
            escaped(said),
            styled(palette::css(prefix.style)),
            escaped(&prefix.content)
        ));
    }

    /// The one component every reference to a bead is drawn by: its glyph,
    /// its id second to its title, and a link to `href` where the page can
    /// open the bead.
    fn reference(&self, html: &mut String, reference: Reference, href: Option<String>) {
        let linked = href.is_some();
        match href {
            Some(href) => html.push_str(&format!("<a class=\"bead\" href=\"{}\">", escaped(&href))),
            None => html.push_str("<span class=\"bead\">"),
        }
        spans(html, &[reference.glyph], Block::Identity, &[], 0);
        html.push_str(&format!(
            "<span{}> </span><span class=\"id\">",
            styled(palette::css(reference.between))
        ));
        spans(html, &reference.id, Block::Identity, &[], 0);
        html.push_str(&format!(
            "</span><span{}>{}</span><span class=\"title\">",
            styled(palette::css(reference.between)),
            indent()
        ));
        spans(html, &reference.title, Block::Identity, &[], 0);
        html.push_str("</span>");
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
        html.push_str("</div>\n");
        for (at, row) in page.rows.iter().enumerate().skip(page.head) {
            if let Some(related) = named.get(&at) {
                let (arrow, rest) = row.split_at(row.len().min(2));
                spans(html, arrow, Block::Identity, &[], 0);
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
                let links = page.links.get(&at).map_or(&[][..], Vec::as_slice);
                spans(html, row, Block::Identity, links, 0);
            }
            html.push('\n');
        }
        html.push_str("</div>");
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
        self.reference(html, reference, key.map(|key| self.asked.opening(&key)));
        true
    }

    /// Every notice the terminal says at its foot, one to a line.
    fn foot(&self, html: &mut String) {
        let notices = notices(self.forest.snapshot(), &[]);
        if notices.is_empty() {
            return;
        }
        html.push_str("<footer>");
        for notice in &notices {
            html.push_str(&format!(
                "<p>{}</p>",
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

/// Spans of one block as HTML, each in its own style, the first of them the
/// block's span `from`. One that `links` names is a link, where it points
/// somewhere a page may link to.
fn spans(html: &mut String, spans: &[Span<'static>], block: Block, links: &[Link], from: usize) {
    for (at, span) in spans.iter().enumerate() {
        let style = styled(palette::css(span.style));
        let text = escaped(&span.content);
        let to = links
            .iter()
            .find(|link| link.block == block && link.at == from + at)
            .map(|link| link.to.as_str())
            .filter(|to| to.starts_with("https://") || to.starts_with("http://"));
        match to {
            Some(to) => html.push_str(&format!("<a href=\"{}\"{style}>{text}</a>", escaped(to))),
            None if style.is_empty() => html.push_str(&text),
            None => html.push_str(&format!("<span{style}>{text}</span>")),
        }
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

/// The page's look: the terminal's monospace grid, and its palette as
/// variables the reader's light or dark preference chooses between.
const STYLE: &str = "\
:root{--bg:#ffffff;--fg:#1d1f21;--shown:#e4e8ec;\
--c0:#1d1f21;--c1:#c82829;--c2:#4d7a00;--c3:#9a6700;--c4:#3465a4;--c5:#8959a8;--c6:#3e8f95;--c7:#5c5f5d;\
--c8:#8e908c;--c9:#d54e53;--c10:#5f8f00;--c11:#b07800;--c12:#4271ae;--c13:#9e6bbd;--c14:#3e999f;--c15:#1d1f21}\
@media (prefers-color-scheme:dark){:root{--bg:#1d1f21;--fg:#d5d8d6;--shown:#33383d;\
--c0:#1d1f21;--c1:#cc6666;--c2:#b5bd68;--c3:#f0c674;--c4:#81a2be;--c5:#b294bb;--c6:#8abeb7;--c7:#c5c8c6;\
--c8:#707880;--c9:#d54e53;--c10:#b9ca4a;--c11:#e7c547;--c12:#7aa6da;--c13:#c397d8;--c14:#70c0b1;--c15:#eaeaea}}\
html{color-scheme:light dark}\
body{margin:0;background:var(--bg);color:var(--fg);\
font:13px/1.45 ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}\
main{display:grid;grid-template-columns:minmax(0,1fr)}\
main.open{grid-template-columns:minmax(0,3fr) minmax(0,2fr)}\
.forest{padding:8px 0;min-width:0;overflow-x:auto}\
.line{display:flex;white-space:pre;padding:0 12px;box-sizing:border-box;width:max-content;min-width:100%}\
.line .state{margin-left:auto;padding-left:2ch;flex:none}\
.line.shown{background:var(--shown)}\
a{color:inherit;text-decoration:none}\
a.fold:hover,a.bead:hover .title,a.close:hover{text-decoration:underline}\
.bead .id{opacity:.75}\
aside{position:sticky;top:0;align-self:start;max-height:100vh;overflow:auto;box-sizing:border-box;\
padding:8px 12px;border-left:1px solid var(--c8)}\
.page{white-space:pre-wrap}\
.close{display:inline-block;margin-bottom:8px}\
footer{padding:8px 12px;color:var(--c3)}\
footer p,.missing{margin:0}\
@media (max-width:720px){main.open{grid-template-columns:minmax(0,1fr)}\
aside{position:static;max-height:none;border-left:0;border-top:1px solid var(--c8)}}";

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    use crate::model::snapshot::{ProviderState, Tree};
    use crate::model::tree::Link;
    use crate::model::types::{Edge, Status};
    use crate::view::address::{address, addressable};
    use crate::view::draw::tests::{counts, drawn_at, frame_of, node, snapshot, tree};
    use crate::view::forest::flatten;
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
        let total = beads.len();
        snapshot(
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
        )
    }

    /// The page is drawn at the terminal's instant, an hour east of UTC, so
    /// a time of day said in UTC would be an hour out.
    fn drawing() -> Drawn {
        Drawn {
            at: drawn_at(),
            offset: FixedOffset::east_opt(3600).expect("an offset"),
            collecting: Vec::new(),
        }
    }

    /// What the terminal says of the project's read, half a minute old, and
    /// what the page says in its place.
    const AGO: &str = "✓ 30s ago";
    const READ_AT: &str = "✓ read 2026-08-30 11:21:44 +01:00";

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
    /// state block apart from the rest as the terminal sets it apart.
    fn page_lines(html: &str) -> Vec<String> {
        (0..lines_html(html).len())
            .map(|at| {
                let line = line_html(html, at).replace("<span class=\"state\">", " ");
                words(&text_of(&line))
            })
            .collect()
    }

    /// Each line of the forest the terminal draws, as a reader sees it, with
    /// the read dated as the page dates it.
    fn terminal_lines(forest: &Forest) -> Vec<String> {
        let rows = frame_of(forest, 160, 60).rows();
        rows[..forest.lines().len()]
            .iter()
            .map(|row| words(row).replace(AGO, READ_AT))
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
            .split("<a class=\"fold\" href=\"")
            .nth(1)
            .expect("the line folds");
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
        assert!(html.contains("class=\"fold\""));
        assert!(!html.contains(":~:"), "{html}");
    }

    #[test]
    fn a_line_with_no_fold_carries_no_id() {
        let html = page_at("/").html;
        let lines = lines_html(&html);
        assert!(lines.iter().any(|line| line.contains("class=\"fold\"")));
        for (at, line) in lines.iter().enumerate() {
            assert_eq!(
                line_id(&html, at).is_some(),
                line.contains("class=\"fold\""),
                "{line}"
            );
        }
    }

    #[test]
    fn the_page_draws_every_line_of_the_forest_as_the_terminal_draws_it() {
        let html = page_at("/").html;
        assert_eq!(page_lines(&html), terminal_lines(&terminal_forest(&[])));
    }

    /// The one difference: a page left open says the same thing for as long
    /// as it is open, so it says when the project was read rather than how
    /// long ago, in the same place and behind the same mark.
    #[test]
    fn a_projects_line_says_the_time_of_day_it_was_read_where_the_terminal_says_how_long_ago() {
        let terminal = frame_of(&terminal_forest(&[]), 160, 60).rows();
        assert_eq!(words(&terminal[0]), format!("▾ summit-works {AGO} 3/9"));
        assert_eq!(
            page_lines(&page_at("/").html)[0],
            format!("▾ summit-works {READ_AT} 3/9")
        );
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
        let close = shown
            .split("<a class=\"close\" href=\"")
            .nth(1)
            .expect("the bead view closes");
        let close = close[..close.find('"').expect("the link ends")].replace("&amp;", "&");

        let mut terminal = flatten(ground());
        assert!(terminal.go_to(&key("smt-4kd3p.7")));
        assert_eq!(page_lines(&page_at(&close).html), terminal_lines(&terminal));
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
        assert!(text_of(aside).contains("○ smt-4kd3p.6  apply the two niri settings"));
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
        assert!(text_of(&answer.html).contains("smt-zzz is in no tree bdi draws"));
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
        let page_rows: Vec<String> = text_of(&view[..view.find("</aside>").unwrap()])
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
            "blocked by: ○ smt-4kd3p.1 wire the niri theme include"
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
            "blocked by: ○ smt-4kd3p.1 wire the niri theme include, ○ smt-4kd3p.8 read the wedge log"
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
            "blocked by: ○ smt-4kd3p.1 wire the niri theme include, ○ smt-4kd3p.8 read the wedge log"
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
            "blocked by: ○ smt-4kd3p.1 wire the niri theme include, ○ smt-4kd3p.8 read the wedge log"
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
            "blocked by: ○ smt-4kd3p.1 wire the niri theme include, ○ smt-4kd3p.8 a bead of another tracker"
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
            "blocked by: ○ smt-4kd3p.1 wire the niri theme include, ○ smt-4kd3p.8 a bead of one tracker, ○ smt-4kd3p.8 a bead of another"
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
}
