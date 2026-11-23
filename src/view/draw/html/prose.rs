//! A bead's description or notes as the page draws them: the markdown its
//! author wrote, read as the terminal reads it, with each line break kept,
//! each bead it names drawn as a reference, and each web address drawn as a
//! short link.
//!
//! The text is anyone's with access to the tracker, so nothing in it is
//! written to the page unescaped. HTML in it is shown as the text it is, and
//! a link is only ever made to a web address.

use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use crate::view::show;

use super::escaped;

/// What a short link sets between an address's site and its last part.
const SITE_TO_PART: &str = " › ";

/// `text` as HTML. `named` draws the reference to the bead an id in the text
/// names, where it names one.
pub(super) fn written(text: &str, named: &dyn Fn(&str) -> Option<String>) -> String {
    let mut writing = Writing {
        html: String::new(),
        said: String::new(),
        named,
        links: Vec::new(),
        in_code: false,
        in_head: false,
        in_html: false,
        source: text,
        taken: None,
    };
    for (event, at) in Parser::new_ext(text, Options::ENABLE_TABLES).into_offset_iter() {
        if !writing.address_takes(&event, &at) {
            writing.take(event);
        }
    }
    writing.write_said();
    writing.html
}

struct Writing<'n> {
    html: String,
    /// Text that has arrived and is not yet written. A run of text can
    /// arrive in several pieces, and an address or an id is only found
    /// whole.
    said: String,
    named: &'n dyn Fn(&str) -> Option<String>,
    /// Each link being written, innermost last: where its text starts in
    /// `html`, and where it goes.
    links: Vec<(usize, String)>,
    in_code: bool,
    in_head: bool,
    /// Whether the text arriving is HTML, which is said as it was written.
    in_html: bool,
    /// The text being written, and the emphasis in it that an address ran
    /// on through and has taken.
    source: &'n str,
    taken: Option<Range<usize>>,
}

impl Writing<'_> {
    /// Whether the event at `at` in the source is part of a web address.
    /// Markdown reads `_draft_` in `https://arkham.example/_draft_/index` as
    /// emphasis, so emphasis straight after the text that has arrived, when
    /// that text ends part-way through an address, is taken into the address
    /// as written, with everything inside it. Nothing that opened before it
    /// or comes after it is taken.
    fn address_takes(&mut self, event: &Event<'_>, at: &Range<usize>) -> bool {
        let inside_taken = |taken: &Range<usize>| taken.start <= at.start && at.end <= taken.end;
        if self.taken.as_ref().is_some_and(inside_taken) {
            return true;
        }
        let word = self.said.rsplit(char::is_whitespace).next().unwrap_or("");
        let address_runs_on = address_in(word).is_some_and(|address| address.end == word.len());
        if !(address_runs_on && matches!(event, Event::Start(Tag::Emphasis | Tag::Strong))) {
            return false;
        }
        self.said.push_str(&self.source[at.clone()]);
        self.taken = Some(at.clone());
        true
    }

    fn take(&mut self, event: Event<'_>) {
        match event {
            Event::Text(text) if self.in_code => self.html.push_str(&escaped(&text)),
            Event::InlineHtml(text) => {
                self.write_said();
                self.said.push_str(&text);
                self.in_html = true;
                self.write_said();
                self.in_html = false;
            }
            Event::Text(text)
            | Event::Html(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::FootnoteReference(text) => self.said.push_str(&text),
            Event::TaskListMarker(done) => self.said.push_str(if done { "[x] " } else { "[ ] " }),
            Event::Start(tag) => {
                self.write_said();
                let opened = self.open(tag);
                self.html.push_str(&opened);
            }
            Event::End(tag) => {
                self.write_said();
                self.close(tag);
            }
            Event::Code(code) => {
                self.write_said();
                self.html
                    .push_str(&format!("<code class=\"code\">{}</code>", escaped(&code)));
            }
            Event::SoftBreak | Event::HardBreak => {
                self.write_said();
                self.html.push_str("<br>");
            }
            Event::Rule => {
                self.write_said();
                self.html.push_str("<hr>");
            }
        }
    }

    fn open(&mut self, tag: Tag<'_>) -> String {
        match tag {
            Tag::Paragraph => "<p>".to_string(),
            Tag::HtmlBlock => {
                self.in_html = true;
                "<p>".to_string()
            }
            // Every level sits under the h3 that heads its section of the page.
            Tag::Heading { .. } => "<h4>".to_string(),
            Tag::BlockQuote(_) => "<blockquote>".to_string(),
            Tag::CodeBlock(_) => {
                self.in_code = true;
                "<pre class=\"code\"><code>".to_string()
            }
            Tag::List(Some(1)) => "<ol>".to_string(),
            Tag::List(Some(first)) => format!("<ol start=\"{first}\">"),
            Tag::List(None) => "<ul>".to_string(),
            Tag::Item => "<li>".to_string(),
            Tag::Emphasis => "<em>".to_string(),
            Tag::Strong => "<strong>".to_string(),
            Tag::Strikethrough => "<s>".to_string(),
            Tag::Table(_) => "<table>".to_string(),
            Tag::TableHead => {
                self.in_head = true;
                "<thead><tr>".to_string()
            }
            Tag::TableRow => "<tr>".to_string(),
            Tag::TableCell if self.in_head => "<th>".to_string(),
            Tag::TableCell => "<td>".to_string(),
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.links.push((self.html.len(), dest_url.to_string()));
                String::new()
            }
            _ => String::new(),
        }
    }

    fn close(&mut self, tag: TagEnd) {
        let closed = match tag {
            TagEnd::Paragraph => "</p>",
            TagEnd::HtmlBlock => {
                self.in_html = false;
                "</p>"
            }
            TagEnd::Heading(_) => "</h4>",
            TagEnd::BlockQuote(_) => "</blockquote>",
            TagEnd::CodeBlock => {
                self.in_code = false;
                "</code></pre>"
            }
            TagEnd::List(true) => "</ol>",
            TagEnd::List(false) => "</ul>",
            TagEnd::Item => "</li>",
            TagEnd::Emphasis => "</em>",
            TagEnd::Strong => "</strong>",
            TagEnd::Strikethrough => "</s>",
            TagEnd::Table => "</table>",
            TagEnd::TableHead => {
                self.in_head = false;
                "</tr></thead>"
            }
            TagEnd::TableRow => "</tr>",
            TagEnd::TableCell if self.in_head => "</th>",
            TagEnd::TableCell => "</td>",
            TagEnd::Link | TagEnd::Image => {
                self.close_link();
                ""
            }
            _ => "",
        };
        self.html.push_str(closed);
    }

    /// The link just written, as a link where it goes to a web address and
    /// as its words and where it goes otherwise, as the terminal says it. A
    /// link whose words are its address is that address's short link. An
    /// image inside a link is only the link's words.
    fn close_link(&mut self) {
        let Some((start, to)) = self.links.pop() else {
            return;
        };
        if !self.links.is_empty() {
            return;
        }
        let words = self.html.split_off(start);
        let says_where_it_goes = words == escaped(&to);
        if is_web_address(&to) {
            if says_where_it_goes {
                self.html.push_str(&short_link(&to));
            } else {
                self.html.push_str(&format!(
                    "<a class=\"link\" href=\"{}\"{NEW_TAB}>{words}</a>",
                    escaped(&to)
                ));
            }
        } else {
            self.html.push_str(&words);
            if !says_where_it_goes {
                self.html.push_str(&escaped(&format!(" ({to})")));
            }
        }
    }

    /// The text that has arrived, each line of it a line of its own. Inside
    /// a link it is the link's words, which name no bead and hold no link,
    /// and HTML holds neither either.
    fn write_said(&mut self) {
        let said = std::mem::take(&mut self.said);
        for (nth, line) in said.trim_end_matches('\n').split('\n').enumerate() {
            if nth > 0 {
                self.html.push_str("<br>");
            }
            if self.links.is_empty() && !self.in_html {
                self.linked(line);
            } else {
                self.html.push_str(&escaped(line));
            }
        }
    }

    /// `text` with each web address in it a short link.
    fn linked(&mut self, text: &str) {
        let mut rest = text;
        while let Some(address) = address_in(rest) {
            self.named(&rest[..address.start]);
            self.html.push_str(&short_link(&rest[address.clone()]));
            rest = &rest[address.end..];
        }
        self.named(rest);
    }

    /// `text` with each id in it that names a bead drawn as its reference.
    fn named(&mut self, text: &str) {
        let mut from = 0;
        for (at, id) in show::ids_in(text) {
            if let Some(reference) = (self.named)(id) {
                self.html.push_str(&escaped(&text[from..at]));
                self.html.push_str(&reference);
                from = at + id.len();
            }
        }
        self.html.push_str(&escaped(&text[from..]));
    }
}

/// What has a link a reader follows open in a new tab, keeping this page
/// where it is, and tells the page it opens nothing of this one.
const NEW_TAB: &str = " target=\"_blank\" rel=\"noopener noreferrer\"";

/// A link to `address` that says its site and its last part, on one line,
/// with the address in full on hover.
fn short_link(address: &str) -> String {
    format!(
        "<a class=\"link address\" href=\"{0}\" title=\"{0}\"{NEW_TAB}>{1}</a>",
        escaped(address),
        escaped(&short(address))
    )
}

/// `address` as its site, without any `www.`, and its last part, where it
/// has one.
fn short(address: &str) -> String {
    let rest = address.split_once("://").map_or(address, |(_, rest)| rest);
    let (site, path) = rest
        .find(['/', '?', '#'])
        .map_or((rest, ""), |at| rest.split_at(at));
    let site = site.rsplit('@').next().unwrap_or(site);
    let site = site.strip_prefix("www.").unwrap_or(site);
    match path.rsplit(['/', '?', '#']).find(|part| !part.is_empty()) {
        Some(last) => format!("{site}{SITE_TO_PART}{last}"),
        None => site.to_string(),
    }
}

/// Whether a link to `to` is one a page may make: to a web address, and
/// never to a script or anything else a browser would run or open.
fn is_web_address(to: &str) -> bool {
    let lower = to.to_ascii_lowercase();
    ["https://", "http://"].into_iter().any(|scheme| {
        lower
            .strip_prefix(scheme)
            .is_some_and(|rest| !rest.starts_with(['/', '?', '#']) && !rest.is_empty())
    })
}

/// Where the first web address in `text` is. It starts a word, and runs to
/// the first space or the first character an address in prose never holds.
/// What ends a sentence after it, a full stop or a closing bracket it never
/// opened, is no part of it.
fn address_in(text: &str) -> Option<Range<usize>> {
    let starts = text
        .match_indices(['h', 'H'])
        .map(|(at, _)| at)
        .filter(|&at| {
            text[at..]
                .get(..4)
                .is_some_and(|s| s.eq_ignore_ascii_case("http"))
        });
    for at in starts {
        let starts_a_word = !text[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric);
        if !starts_a_word {
            continue;
        }
        let end = text[at..]
            .find(|c: char| c.is_whitespace() || "<>\"`".contains(c))
            .map_or(text.len(), |end| at + end);
        let address = without_what_ends_a_sentence(&text[at..end]);
        if is_web_address(address) {
            return Some(at..at + address.len());
        }
    }
    None
}

fn without_what_ends_a_sentence(address: &str) -> &str {
    let mut address = address;
    loop {
        let unopened = |open: char, close: char| {
            address.ends_with(close)
                && address.matches(open).count() < address.matches(close).count()
        };
        if address.ends_with(['.', ',', ';', ':', '!', '?', '\''])
            || unopened('(', ')')
            || unopened('[', ']')
        {
            address = &address[..address.len() - 1];
        } else {
            return address;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Each bead the tests name, by the reference `named` draws for it.
    fn named(id: &str) -> Option<String> {
        ["lgt-4.2", "lgt-4.4.2"]
            .contains(&id)
            .then(|| format!("<a class=\"bead\">{id}</a>"))
    }

    fn html(text: &str) -> String {
        written(text, &named)
    }

    /// An author who puts a line of its own under another means it to stand
    /// apart, as a question bead's Options and Recommendation do, so a line
    /// break is kept rather than read as a space.
    #[test]
    fn each_line_the_author_wrote_stays_a_line_of_its_own() {
        assert_eq!(
            html("Options: re-silver or replace\nRecommendation: re-silver\n\nBecause it is cheaper."),
            "<p>Options: re-silver or replace<br>Recommendation: re-silver</p>\
             <p>Because it is cheaper.</p>"
        );
    }

    #[test]
    fn a_bead_the_prose_names_is_drawn_by_the_reference_it_is_given() {
        assert_eq!(
            html("Run it once lgt-4.2 is done, and log it in lgt-4.4.2."),
            "<p>Run it once <a class=\"bead\">lgt-4.2</a> is done, \
             and log it in <a class=\"bead\">lgt-4.4.2</a>.</p>"
        );
    }

    /// An id shaped like a bead's that names none is the text it is.
    #[test]
    fn an_id_that_names_no_bead_is_left_as_text() {
        assert_eq!(html("See lgt-9 first."), "<p>See lgt-9 first.</p>");
    }

    #[test]
    fn a_web_address_is_a_short_link_saying_its_site_and_last_part() {
        assert_eq!(
            html(
                "The procedure is at https://www.trinity-works.example/manuals/fog/diaphone-test."
            ),
            "<p>The procedure is at <a class=\"link address\" \
             href=\"https://www.trinity-works.example/manuals/fog/diaphone-test\" \
             title=\"https://www.trinity-works.example/manuals/fog/diaphone-test\" \
             target=\"_blank\" rel=\"noopener noreferrer\">\
             trinity-works.example › diaphone-test</a>.</p>"
        );
    }

    #[test]
    fn an_address_shortens_to_its_site_and_the_last_part_it_has() {
        assert_eq!(short("https://dunwich.example"), "dunwich.example");
        assert_eq!(short("http://dunwich.example/"), "dunwich.example");
        assert_eq!(
            short("https://keeper@dunwich.example:8080/log/"),
            "dunwich.example:8080 › log"
        );
        assert_eq!(
            short("https://chat.arkham.example/#narrow/channel/9-lamps/near/1204"),
            "chat.arkham.example › 1204"
        );
        assert_eq!(
            short("https://arkham.example/find?q=lamp"),
            "arkham.example › q=lamp"
        );
    }

    /// A bead named inside an address is part of the address, not a bead.
    #[test]
    fn an_id_inside_an_address_is_part_of_the_address() {
        let drawn = html("https://arkham.example/beads/lgt-4.2");
        assert!(!drawn.contains("class=\"bead\""), "{drawn}");
        assert!(drawn.contains("arkham.example › lgt-4.2</a>"), "{drawn}");
    }

    #[test]
    fn what_ends_a_sentence_after_an_address_is_no_part_of_it() {
        for (text, address) in [
            ("(see https://arkham.example/a)", "https://arkham.example/a"),
            (
                "https://arkham.example/wiki/Lamp_(oil), then",
                "https://arkham.example/wiki/Lamp_(oil)",
            ),
            ("at https://arkham.example/a.", "https://arkham.example/a"),
            ("'https://arkham.example/a'", "https://arkham.example/a"),
        ] {
            let found = address_in(text).map(|at| &text[at]);
            assert_eq!(found, Some(address), "{text}");
        }
    }

    #[test]
    fn only_a_web_address_that_starts_a_word_is_one() {
        for text in [
            "xhttps://arkham.example",
            "https://",
            "ftp://arkham.example",
            "mailto:keeper@arkham.example",
        ] {
            assert_eq!(address_in(text), None, "{text}");
        }
        assert_eq!(address_in("HTTPS://arkham.example"), Some(0..22));
    }

    /// A link the author gave words to keeps them. One whose words are its
    /// address, as an address in angle brackets is, is its short link.
    #[test]
    fn a_link_keeps_its_authors_words_and_an_address_in_brackets_is_short() {
        assert_eq!(
            html("Read [the manual](https://arkham.example/manual)."),
            "<p>Read <a class=\"link\" href=\"https://arkham.example/manual\" \
             target=\"_blank\" rel=\"noopener noreferrer\">the manual</a>.</p>"
        );
        assert!(html("<https://arkham.example/manual>").contains(">arkham.example › manual</a>"));
    }

    /// The words of a link hold no link of their own, and name no bead.
    #[test]
    fn a_links_words_hold_neither_a_link_nor_a_bead() {
        let drawn = html("[lgt-4.2 at https://arkham.example/a](https://arkham.example/b)");
        assert_eq!(drawn.matches("<a ").count(), 1, "{drawn}");
        let drawn =
            html("[![the map](https://arkham.example/map.png)](https://arkham.example/maps)");
        assert_eq!(
            drawn,
            "<p><a class=\"link\" href=\"https://arkham.example/maps\" \
             target=\"_blank\" rel=\"noopener noreferrer\">the map</a></p>"
        );
    }

    /// A link to anything but a web address is no link: a script given as
    /// one would run when followed. Its words are said, and where it goes,
    /// as the terminal says it.
    #[test]
    fn a_link_to_anything_but_a_web_address_is_its_words_and_where_it_goes() {
        assert_eq!(
            html("[click](javascript:alert(1))"),
            "<p>click (javascript:alert(1))</p>"
        );
        assert_eq!(html("[read](https:///etc)"), "<p>read (https:///etc)</p>");
    }

    /// HTML in the text is shown as the text it is: never passed to the
    /// page, and never dropped.
    #[test]
    fn html_in_the_text_is_shown_as_text() {
        let block = html("<script>alert(\"lamp\")</script>\n\nafter");
        assert_eq!(
            block,
            "<p>&lt;script&gt;alert(&quot;lamp&quot;)&lt;/script&gt;</p><p>after</p>"
        );
        assert_eq!(
            html("a <b onclick=\"x\">bold</b> word"),
            "<p>a &lt;b onclick=&quot;x&quot;&gt;bold&lt;/b&gt; word</p>"
        );
        assert_eq!(
            html("Trim it & <i>then</i> 'light' it"),
            "<p>Trim it &amp; &lt;i&gt;then&lt;/i&gt; &#39;light&#39; it</p>"
        );
    }

    /// HTML is shown as it was written, so an address or an id inside it
    /// is no link and no reference.
    #[test]
    fn html_in_the_text_holds_no_link_or_bead() {
        for text in [
            "<div title=\"https://arkham.example/docs\">\nlgt-4.2\n</div>",
            "Write <a href=\"https://arkham.example/docs\"> for it.",
        ] {
            let drawn = html(text);
            assert!(!drawn.contains("<a "), "{drawn}");
            assert!(
                drawn.contains("https://arkham.example/docs&quot;"),
                "{drawn}"
            );
        }
    }

    /// A bare address holding what markdown would read as emphasis keeps
    /// it, so the link goes where the address says.
    #[test]
    fn a_bare_address_keeps_what_markdown_reads_as_emphasis() {
        let drawn = html("See https://arkham.example/docs/_draft_/index and *this*.");
        assert!(
            drawn.contains("href=\"https://arkham.example/docs/_draft_/index\""),
            "{drawn}"
        );
        assert!(drawn.contains("<em>this</em>"), "{drawn}");
        let drawn = html("https://arkham.example/a*b*c/**d**");
        assert!(
            drawn.contains("href=\"https://arkham.example/a*b*c/**d**\""),
            "{drawn}"
        );
    }

    /// An address taking emphasis takes nothing that opened before it, so
    /// every element around it closes, however deep it sits.
    #[test]
    fn an_address_taking_emphasis_leaves_every_element_around_it_closed() {
        let address = "https://arkham.example/_a_/b";
        for text in [
            address.to_string(),
            format!("Read {address} now."),
            format!("### {address}"),
            format!("**{address}**"),
            format!("- {address}"),
            format!("- dunwich\n  - {address}"),
            format!("> {address}"),
            format!("> - [{address}](https://arkham.example/c)"),
            format!("[{address}](https://arkham.example/c)"),
            format!("| where |\n| --- |\n| {address} |"),
            "https://arkham.example/_a_".to_string(),
        ] {
            let drawn = html(&text);
            assert_eq!(unbalanced(&drawn), None, "{text}\n{drawn}");
            assert!(drawn.contains("_a_"), "{text}\n{drawn}");
        }
    }

    /// Every bead description and note the fixtures hold is drawn as HTML
    /// whose elements all close.
    #[test]
    fn every_fixtures_prose_is_drawn_with_every_element_closed() {
        let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
        let mut drawn = 0;
        for file in std::fs::read_dir(fixtures).unwrap() {
            let path = file.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let values: Vec<serde_json::Value> = text
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .chain(serde_json::from_str(&text).ok())
                .collect();
            for prose in values.iter().flat_map(prose_in) {
                let written = html(prose);
                assert_eq!(unbalanced(&written), None, "{}\n{written}", path.display());
                drawn += 1;
            }
        }
        assert!(drawn > 0, "no description in {fixtures}");
    }

    /// Each description and note in `value`, however deep.
    fn prose_in(value: &serde_json::Value) -> Vec<&str> {
        match value {
            serde_json::Value::Object(fields) => fields
                .iter()
                .flat_map(|(key, field)| match (key.as_str(), field.as_str()) {
                    ("description" | "notes", Some(prose)) => vec![prose],
                    _ => prose_in(field),
                })
                .collect(),
            serde_json::Value::Array(items) => items.iter().flat_map(prose_in).collect(),
            _ => Vec::new(),
        }
    }

    /// The first tag in `html` that closes nothing open, or one left open.
    fn unbalanced(html: &str) -> Option<String> {
        let mut open = Vec::new();
        for tag in html.split('<').skip(1) {
            let Some(end) = tag.find('>') else {
                return Some(format!("<{tag}"));
            };
            let name = tag[..end].split(' ').next().unwrap_or_default();
            if let Some(closing) = name.strip_prefix('/') {
                if open.pop() != Some(closing) {
                    return Some(format!("</{closing}>"));
                }
            } else if !["br", "hr"].contains(&name) {
                open.push(name);
            }
        }
        open.pop().map(|name| format!("<{name}> left open"))
    }

    /// An address carrying a quote is cut at it, so nothing in an address
    /// can close the attribute it is written in.
    #[test]
    fn an_address_never_closes_its_attribute() {
        let drawn = html("https://arkham.example/a\"onmouseover=\"x");
        assert!(
            drawn.contains("href=\"https://arkham.example/a\""),
            "{drawn}"
        );
        assert!(!drawn.contains("\"onmouseover"), "{drawn}");
    }

    #[test]
    fn code_is_marked_as_code_and_holds_no_link_or_bead() {
        assert_eq!(
            html("Trim the `wick` daily."),
            "<p>Trim the <code class=\"code\">wick</code> daily.</p>"
        );
        assert_eq!(
            html("```\nlog lgt-4.2 at https://arkham.example\n<b>\n```"),
            "<pre class=\"code\"><code>log lgt-4.2 at https://arkham.example\n&lt;b&gt;\n</code></pre>"
        );
    }

    #[test]
    fn markdown_is_drawn_as_the_elements_it_stands_for() {
        assert_eq!(
            html("### Steps\n\n1. *grease* it\n2. **light** it\n\n- one\n\n> quoted\n\n---"),
            "<h4>Steps</h4><ol><li><em>grease</em> it</li><li><strong>light</strong> it</li></ol>\
             <ul><li>one</li></ul><blockquote><p>quoted</p></blockquote><hr>"
        );
        assert_eq!(
            html("3. third\n4. fourth"),
            "<ol start=\"3\"><li>third</li><li>fourth</li></ol>"
        );
        assert_eq!(
            html("| lamp | burns |\n|---|---|\n| oil | 6h |"),
            "<table><thead><tr><th>lamp</th><th>burns</th></tr></thead>\
             <tr><td>oil</td><td>6h</td></tr></table>"
        );
    }
}
