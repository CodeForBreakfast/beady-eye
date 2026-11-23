//! Every colour and weight `bdi` draws, named for what it means rather than
//! for what it is, and the only place in the view either is spelled.
//!
//! A slot is a `Style` and not a `Color`, which is what lets one be the
//! terminal's own foreground plus a weight rather than a colour of its own.
//! Two slots holding one value stay apart where they are two claims: a change
//! to what a grey means then reaches the claim it was made about and no other.
//!
//! `src/view/sgr.rs` is the one module outside this one that names a colour.
//! It replays a pane's own escapes rather than choosing anything, so the
//! colours it names are the pane's and no palette can hold them.

use std::str::FromStr;

use ratatui::style::{Color, Modifier, Style};

use crate::config::{Background, Slot};

// ---- `bd`'s own, quoted ------------------------------------------------

/// `bd list`'s colours for a status, read off `bd` 1.2.2's output. They are
/// literal rather than named because `bd`'s are: it sends 24-bit values that
/// do not move with the terminal's theme, so a named colour here would track
/// the theme away from the tool this is matching.
pub(crate) const STATUS_IN_PROGRESS: Style = Style::new().fg(Color::Rgb(255, 180, 84));
pub(crate) const STATUS_BLOCKED: Style = Style::new().fg(Color::Rgb(242, 109, 120));
pub(crate) const STATUS_CLOSED: Style = Style::new().fg(Color::Rgb(128, 144, 160));
pub(crate) const STATUS_DEFERRED: Style = Style::new().fg(Color::Rgb(108, 118, 128));
/// Read off `bd` 1.3.0's source rather than its output: `ColorStatusPinned`,
/// the same value on a light background as a dark one.
pub(crate) const STATUS_PINNED: Style = Style::new().fg(Color::Rgb(210, 166, 255));
/// `ColorStatusHooked`, read the same way.
pub(crate) const STATUS_HOOKED: Style = Style::new().fg(Color::Rgb(89, 194, 255));

/// `bd` sends no escape at all for an open bead, and a glyph that inherits is
/// what lets the brightness of the row it sits on reach it.
pub(crate) const STATUS_OPEN: Style = Style::new();

/// `bd show`'s colour for the id at the head of the page, literal for the
/// reason the statuses are.
pub(crate) const IDENTITY: Style = Style::new().fg(Color::Rgb(89, 194, 255));

// ---- the two axes `bd` cannot draw -------------------------------------

/// A live agent is here.
pub(crate) const AGENT: Style = Style::new().fg(Color::Green);

/// This wants looking at.
pub(crate) const ATTENTION: Style = Style::new().fg(Color::Yellow);

// ---- how live a row is -------------------------------------------------

/// A row an agent is on: the ground and a weight.
///
/// A theme's own foreground is routinely the same value as its colour 7 or
/// its colour 15, so there is no colour above the ground to reach for. A
/// weight is what is left, and it is the one treatment here whose size is the
/// reader's font rather than the reader's theme: the brightening a terminal
/// does on bold is a remap of palette slots 0-7, and the default foreground
/// has no slot to remap.
pub(crate) const TIER_STAFFED: Style = Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD);

/// Nobody on it and still going, which is most of the forest most of the
/// time: the terminal's own foreground, untreated. It is the ground the other
/// two are measured from rather than a rung between them, so the commonest
/// row on the page is the one the scale spends nothing on.
pub(crate) const TIER_OPEN: Style = Style::new().fg(Color::Reset);

/// Finished, nobody on it: one rung under the ground, at the theme's colour
/// 8, which every theme sets and few rows on the page otherwise use. Named
/// rather than literal because a rung pinned to a number keeps no distance
/// from a ground it never sees — only the theme knows what its foreground is,
/// so only a value the theme also chooses can stay a fixed way from it.
pub(crate) const TIER_FINISHED: Style = Style::new().fg(Color::DarkGray);

// ---- chrome, and content that is not a row -----------------------------

/// Box-drawing and fold arrows: how the tree is shaped rather than how a bead
/// is going, so it is held on the ground while the row around it steps off
/// it. Naming the colour is not enough to stay there — a tone is patched
/// under the whole row and a weight in one composes with a weight in the
/// other, where a colour replaces it — so this says the weight it does not
/// take as well as the colour it does.
pub(crate) const STRUCTURE: Style = Style::new()
    .fg(Color::Reset)
    .remove_modifier(Modifier::BOLD);

/// Metadata, chrome, an affordance, a rule.
pub(crate) const QUIET: Style = Style::new().fg(Color::DarkGray);

/// In the tail band only: these are `bdi`'s words and not the pane's.
/// `design.md`'s account of the band makes this the whole of what tells the
/// two apart, so it is the one tone on the screen with nothing beside it.
///
/// The one slot here the reader's background selects, and the reason there
/// is a `[theme]` key at all. Every other slot is a value the reader's own
/// theme resolves — its foreground, or one of its sixteen — or one of
/// `bd`'s, which are absolute so that a status is the colour in `bdi` that
/// it is in `bd`. Neither kind has a light form to choose. This one is a
/// treatment `bdi` composes itself, and the terminal resolves it against
/// the background rather than against the palette.
///
/// **Dim over the default foreground is the composition that inverts.** It
/// is git's `GIT_COLOR_FAINT_DEFAULT`, SGR `2;39`, and the terminals that
/// implement dim by scaling the foreground toward black — xterm, VTE,
/// Alacritty, Windows Terminal — leave a light theme's near-black
/// foreground darker than plain text rather than fainter. Windows
/// Terminal's issue #16493 measured it: faint is "the darkest of the three
/// in each line", which against white is bold by another name. A band whose
/// whole distinction is that `bdi` speaks under the pane becomes one where
/// it shouts over it.
///
/// So a dark background is answered with the attribute, which a reader with
/// `NO_COLOR` set still has, and a light one at colour 8, where the theme's
/// own choice of a tone between its foreground and its background carries
/// the distinction and no arithmetic of the terminal's is involved. Not a
/// slant and not an underline: italic and `CODE`'s cyan are prose's
/// namespace, an underline says a link wherever it is drawn, and the band is
/// `bdi` scanning its own words at the reader rather than prose to read or
/// somewhere to go.
///
/// **A light background and `NO_COLOR` together leave the band no tone**,
/// because the one channel that survives colour being off is the one a
/// light background inverts. Such a reader has the rule and which of the
/// two states the band is in, and nothing else says whose words a row is.
pub(crate) fn voice(background: Background) -> Style {
    match background {
        Background::Dark => Style::new().fg(Color::Reset).add_modifier(Modifier::DIM),
        Background::Light => Style::new().fg(Color::DarkGray),
    }
}

/// Every row of the bead window: the head the reader came for, and under it
/// the facts, the prose and the related rows. The terminal's own foreground,
/// which is to say the window spends no brightness on its own structure —
/// what stands out on the page takes a weight, and *dim* is left to mean one
/// thing on every surface. It is a claim about a page of text rather than
/// about a row, so the forest's scale can move without the window following
/// it.
pub(crate) const PAGE: Style = Style::new().fg(Color::Reset);

/// `bdi`'s own sentence about the forest where nothing went wrong in it: the
/// hidden trees the filter is holding back, and a forest that was read and
/// held no work. Said at the terminal's own foreground because the thing it
/// contrasts with is `ATTENTION`.
pub(crate) const PLAIN: Style = Style::new().fg(Color::Reset);

/// A code span or a code block: prose's own namespace.
pub(crate) const CODE: Style = Style::new().fg(Color::Cyan);

// ---- weights -----------------------------------------------------------

// A weight says *this is the thing to go to*, and on both surfaces that is
// one meaning rather than two: a row with an agent on it is where the reader
// is heading, and so is the name of the section they are looking for on a
// page of text. What a reader learns on one surface holds on the next, which
// is the whole of why a channel may be spent twice.
//
// On the forest it is also the whole of the top rung of the liveness scale.
// A staffed row and an unworked one are both at the terminal's own
// foreground, so nothing else separates them: the scale spends the channel
// because it has run out of brightness at the ground.
//
// Inside the window it makes no further distinction, and the slots below say
// so by holding one value. What tells a section name from a heading from a
// strong word is position — the first two own their row and the third sits
// inside a sentence — rather than the treatment. They stay apart because
// they quote different sources: prose's markup can take a tone without
// `bd show`'s section names following it.

/// The row under the cursor, drawn so the eye finds it without reading it.
pub(crate) const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);

/// A window's own name, on its border.
pub(crate) const TITLE: Style = Style::new().add_modifier(Modifier::BOLD);

/// `bd show`'s section names, which stand out from a page of text: a weight
/// and no tone, so the page reaches them as it reaches everything else on it.
pub(crate) const SECTION: Style = Style::new().add_modifier(Modifier::BOLD);

/// A heading in prose. A weight and no colour, so it composes onto the tone
/// of the page it is drawn on.
pub(crate) const HEADING: Style = Style::new().add_modifier(Modifier::BOLD);

/// Prose's own emphasis, each a weight alone for the reason a heading is.
pub(crate) const EMPHASIS: Style = Style::new().add_modifier(Modifier::ITALIC);
pub(crate) const STRONG: Style = Style::new().add_modifier(Modifier::BOLD);

/// Somewhere to go: a reference in prose, and a badge whose config gave it a
/// `link`.
pub(crate) const LINK: Style = Style::new().add_modifier(Modifier::UNDERLINED);

// ---- what a config may name --------------------------------------------

/// The slot a config named, for a badge a reader asked to be drawn in one.
pub(crate) fn slot(slot: Slot) -> Style {
    match slot {
        Slot::StatusOpen => STATUS_OPEN,
        Slot::StatusInProgress => STATUS_IN_PROGRESS,
        Slot::StatusBlocked => STATUS_BLOCKED,
        Slot::StatusClosed => STATUS_CLOSED,
        Slot::StatusDeferred => STATUS_DEFERRED,
        Slot::Identity => IDENTITY,
        Slot::Agent => AGENT,
        Slot::Attention => ATTENTION,
        Slot::TierStaffed => TIER_STAFFED,
        Slot::TierOpen => TIER_OPEN,
        Slot::TierFinished => TIER_FINISHED,
        Slot::Structure => STRUCTURE,
        Slot::Quiet => QUIET,
        Slot::Page => PAGE,
        Slot::Plain => PLAIN,
        Slot::Code => CODE,
        Slot::Selected => SELECTED,
        Slot::Title => TITLE,
        Slot::Section => SECTION,
        Slot::Heading => HEADING,
        Slot::Emphasis => EMPHASIS,
        Slot::Strong => STRONG,
        Slot::Link => LINK,
    }
}

/// The colour a config named, for a badge a reader asked to be drawn in one.
///
/// The one colour this module hands out that `bdi` did not choose. It is named
/// here rather than where it is drawn so that this stays the only place in the
/// view a colour is spelled, whoever spelled it.
pub(crate) fn absolute(colour: Color) -> Style {
    Style::new().fg(colour)
}

/// A colour as a config wrote it, in any form `Color` names one by: one of the
/// sixteen by name, `#rrggbb`, or an index into the terminal's palette.
///
/// Reading a colour is what this module is for, so the reading is here even
/// though what asks for it is `config` refusing a name as it reads the file.
/// `colours-come-from-the-palette` holds the whole of `Color`'s vocabulary to
/// this module, and `from_str` is part of it.
pub(crate) fn colour_named(written: &str) -> Option<Color> {
    Color::from_str(written).ok()
}

// ---- on a web page ------------------------------------------------------

/// The web page's colours on a light page, as CSS custom properties. Every
/// colour the page's stylesheet names is one of these, so a light and a dark
/// value are all a new colour needs.
///
/// - `--surface` is the page and `--fg` its text. `--sunk` is a ground set
///   into the page and `--raised` one lifted off it, such as a menu.
/// - `--muted` is secondary text and `--faint` the quietest, such as an id.
/// - `--line` is a rule or a border, and `--rail` a tree's depth line.
/// - `--accent` is somewhere to go or what is chosen, `--accent-soft` a
///   ground in it and `--on-accent` text over it. `--sel` is the row the
///   reader came to.
/// - `--warn` wants looking at, over `--warn-soft`. `--ok` is live or done.
/// - `--status-*` is a bead's status, in `bd`'s hues made legible on each
///   ground, drawn on its status mark alone. In-progress is blue where `bd`'s
///   is orange, because amber on the page means `--warn` and nothing else.
const PAGE_LIGHT: &str = "\
--surface:#ffffff;--sunk:#f3f5f8;--raised:#ffffff;--fg:#17202a;--muted:#5d6874;--faint:#8b96a2;\
--line:#e2e6eb;--rail:#cfd6dd;--accent:#0b6e82;--accent-soft:#e1f1f4;--on-accent:#ffffff;--sel:#e9f3f6;\
--warn:#a85700;--warn-soft:#fdf1e3;--ok:#2b8a57;--shadow:0 10px 30px rgba(23,32,42,.14);\
--status-open:#46525e;--status-in-progress:#2f6bd8;--status-blocked:#c92a3a;--status-closed:#84919d;\
--status-deferred:#6e62a8;--ink:#17202a";

/// The same colours on a dark page.
const PAGE_DARK: &str = "\
--surface:#141b21;--sunk:#1a2229;--raised:#1d262e;--fg:#e3e8ec;--muted:#9aa6b1;--faint:#6c7884;\
--line:#263039;--rail:#34404a;--accent:#5cc6d4;--accent-soft:#143840;--on-accent:#0c1115;--sel:#17323a;\
--warn:#f0b35a;--warn-soft:#33281a;--ok:#5cc28a;--shadow:0 10px 30px rgba(0,0,0,.5);\
--status-open:#c3ccd4;--status-in-progress:#6ea8ff;--status-blocked:#f0707a;--status-closed:#66737f;\
--status-deferred:#a99ee0;--ink:#0c1115";

/// The terminal's sixteen colours as the page's, which is what a pane's own
/// escapes and a slot naming one of the sixteen resolve to. `--ink` is the
/// darkest colour of each theme, for the terminal's black.
const PAGE_SIXTEEN: &str = "\
--c0:var(--ink);--c1:var(--status-blocked);--c2:var(--ok);--c3:var(--warn);--c4:var(--accent);\
--c5:var(--status-deferred);--c6:var(--accent);--c7:var(--muted);--c8:var(--muted);\
--c9:var(--status-blocked);--c10:var(--ok);--c11:var(--warn);--c12:var(--accent);\
--c13:var(--status-deferred);--c14:var(--accent);--c15:var(--fg)";

/// The page's colours as a stylesheet, light unless the reader prefers dark.
pub(crate) fn page_colours() -> String {
    format!(
        ":root{{{PAGE_LIGHT};{PAGE_SIXTEEN}}}\
         @media (prefers-color-scheme:dark){{:root{{{PAGE_DARK}}}}}"
    )
}

/// The page's own colour for each of `bd`'s values it draws. `bd`'s are
/// pitched for a dark terminal, and the page's keep each hue legible on a
/// light ground as well as a dark one.
const PAGE_TOKENS: [(Style, &str); 5] = [
    (STATUS_IN_PROGRESS, "--status-in-progress"),
    (STATUS_BLOCKED, "--status-blocked"),
    (STATUS_CLOSED, "--status-closed"),
    (STATUS_DEFERRED, "--status-deferred"),
    (IDENTITY, "--accent"),
];

/// A style as CSS declarations for an element drawn over its parent's, as a
/// span is drawn over its row's: what the style leaves alone is left out.
///
/// The terminal's own foreground and its sixteen colours are the page's
/// `--fg` and `--c0` to `--c15`, and `bd`'s statuses are the page's
/// `--status-*`, so the page's own theme resolves them.
pub(crate) fn css(style: Style) -> String {
    declarations(style, true)
}

/// `bd`'s colours for a status, which the page draws on a status mark and
/// nowhere else. A hooked bead's is left out because it is also
/// `IDENTITY`'s, which the page draws as its own accent.
const STATUSES: [Style; 5] = [
    STATUS_IN_PROGRESS,
    STATUS_BLOCKED,
    STATUS_CLOSED,
    STATUS_DEFERRED,
    STATUS_PINNED,
];

/// A style as [`css`] gives it, for anything on the page but a status mark:
/// a status's colour is left out, so the words take their parent's.
pub(crate) fn css_without_status(style: Style) -> String {
    if STATUSES.iter().any(|status| status.fg == style.fg) {
        css(Style { fg: None, ..style })
    } else {
        css(style)
    }
}

/// A style as [`css`] gives it, for a style whose colour a config wrote: a
/// 24-bit colour is drawn as written even where it is one of `bd`'s, because
/// the page cannot otherwise tell it from that status.
pub(crate) fn css_as_written(style: Style) -> String {
    declarations(style, false)
}

/// A style as CSS, with `bd`'s colours as the page's own where `themed`.
fn declarations(style: Style, themed: bool) -> String {
    let mut said = Vec::new();
    if let Some(colour) = style.fg {
        said.push(format!("color:{}", css_colour(colour, "--fg", themed)));
    }
    if let Some(colour) = style.bg {
        said.push(format!(
            "background:{}",
            css_colour(colour, "--surface", themed)
        ));
    }
    let added = style.add_modifier;
    let removed = style.sub_modifier;
    for (modifier, on, off) in [
        (Modifier::BOLD, "font-weight:bold", "font-weight:normal"),
        (Modifier::DIM, "opacity:.6", "opacity:1"),
        (Modifier::ITALIC, "font-style:italic", "font-style:normal"),
    ] {
        if added.contains(modifier) {
            said.push(on.to_string());
        } else if removed.contains(modifier) {
            said.push(off.to_string());
        }
    }
    let lines: Vec<&str> = [
        (Modifier::UNDERLINED, "underline"),
        (Modifier::CROSSED_OUT, "line-through"),
    ]
    .into_iter()
    .filter(|(modifier, _)| added.contains(*modifier))
    .map(|(_, line)| line)
    .collect();
    if !lines.is_empty() {
        said.push(format!("text-decoration:{}", lines.join(" ")));
    }
    if added.contains(Modifier::REVERSED) {
        said.push("background:var(--fg);color:var(--surface)".to_string());
    }
    said.join(";")
}

/// One colour as CSS, with the terminal's default standing for `default`, and
/// `bd`'s colours as the page's own where `themed`.
fn css_colour(colour: Color, default: &str, themed: bool) -> String {
    let slot = |n: u8| format!("var(--c{n})");
    if let Some((_, token)) = PAGE_TOKENS
        .iter()
        .filter(|_| themed)
        .find(|(style, _)| style.fg == Some(colour))
    {
        return format!("var({token})");
    }
    match colour {
        Color::Reset => format!("var({default})"),
        Color::Black => slot(0),
        Color::Red => slot(1),
        Color::Green => slot(2),
        Color::Yellow => slot(3),
        Color::Blue => slot(4),
        Color::Magenta => slot(5),
        Color::Cyan => slot(6),
        Color::Gray => slot(7),
        Color::DarkGray => slot(8),
        Color::LightRed => slot(9),
        Color::LightGreen => slot(10),
        Color::LightYellow => slot(11),
        Color::LightBlue => slot(12),
        Color::LightMagenta => slot(13),
        Color::LightCyan => slot(14),
        Color::White => slot(15),
        Color::Indexed(n) if n < 16 => slot(n),
        Color::Indexed(n) => {
            let (r, g, b) = xterm_256(n);
            format!("#{r:02x}{g:02x}{b:02x}")
        }
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
    }
}

/// Where xterm puts an index past the sixteen: a six-level cube, then a ramp
/// of greys.
fn xterm_256(n: u8) -> (u8, u8, u8) {
    if n >= 232 {
        let grey = 8 + (n - 232) * 10;
        return (grey, grey, grey);
    }
    let level = |step: u8| if step == 0 { 0 } else { 55 + step * 40 };
    let cube = n - 16;
    (level(cube / 36), level(cube / 6 % 6), level(cube % 6))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::Background;

    /// A status takes the page's own colour for it, and the terminal's own
    /// colours become the page's, so the page's theme resolves both.
    #[test]
    fn a_style_on_the_page_takes_the_pages_palette() {
        assert_eq!(css(STATUS_IN_PROGRESS), "color:var(--status-in-progress)");
        assert_eq!(css(STATUS_BLOCKED), "color:var(--status-blocked)");
        assert_eq!(css(STATUS_CLOSED), "color:var(--status-closed)");
        assert_eq!(css(STATUS_DEFERRED), "color:var(--status-deferred)");
        assert_eq!(css(TIER_FINISHED), "color:var(--c8)");
        assert_eq!(css(STRUCTURE), "color:var(--fg);font-weight:normal");
        assert_eq!(css(TIER_STAFFED), "color:var(--fg);font-weight:bold");
        assert_eq!(css(STATUS_OPEN), "");
    }

    /// A colour a config wrote is drawn as written, even where it is one of
    /// `bd`'s, while a colour the terminal's theme names still takes the
    /// page's theme.
    #[test]
    fn a_style_as_written_keeps_its_24_bit_colour() {
        assert_eq!(css_as_written(STATUS_BLOCKED), "color:#f26d78");
        assert_eq!(css_as_written(IDENTITY), "color:#59c2ff");
        assert_eq!(css_as_written(absolute(Color::Red)), "color:var(--c1)");
    }

    /// A colour the light theme sets and the dark one leaves out would draw
    /// a light value on a dark page.
    #[test]
    fn every_colour_on_the_page_is_set_for_light_and_for_dark() {
        let names = |theme: &str| -> Vec<String> {
            let mut names: Vec<String> = theme
                .split(';')
                .filter_map(|declaration| declaration.split_once(':'))
                .map(|(name, _)| name.to_string())
                .collect();
            names.sort();
            names
        };
        assert!(names(PAGE_LIGHT).contains(&"--surface".to_string()));
        assert_eq!(names(PAGE_LIGHT), names(PAGE_DARK));
        assert!(page_colours().contains("prefers-color-scheme:dark"));
    }

    /// A closed bead's mark is filled like a blocked one's, so its tick is
    /// what tells the two apart, and it is drawn in the page's own colour over
    /// the closed colour. WCAG asks 3:1 of a graphic a reader has to make out.
    #[test]
    fn a_closed_marks_tick_stands_out_from_the_closed_colour_in_both_themes() {
        let token = |theme: &str, name: &str| -> (f64, f64, f64) {
            let value = theme
                .split(';')
                .find_map(|declaration| declaration.strip_prefix(&format!("{name}:#")))
                .unwrap_or_else(|| panic!("{name} is a colour"));
            let channel = |at: usize| {
                let c = f64::from(u8::from_str_radix(&value[at..at + 2], 16).expect("hex")) / 255.0;
                if c <= 0.039_28 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            (channel(0), channel(2), channel(4))
        };
        let luminance = |(r, g, b): (f64, f64, f64)| 0.2126 * r + 0.7152 * g + 0.0722 * b;
        for (name, theme) in [("light", PAGE_LIGHT), ("dark", PAGE_DARK)] {
            let tick = luminance(token(theme, "--surface"));
            let fill = luminance(token(theme, "--status-closed"));
            let contrast = (tick.max(fill) + 0.05) / (tick.min(fill) + 0.05);
            assert!(contrast >= 3.0, "{name}: {contrast:.2}:1");
        }
    }

    /// Past the sixteen, an index is the colour xterm puts there.
    #[test]
    fn an_index_past_the_sixteen_is_xterms_colour() {
        assert_eq!(xterm_256(16), (0, 0, 0));
        assert_eq!(xterm_256(196), (255, 0, 0));
        assert_eq!(xterm_256(231), (255, 255, 255));
        assert_eq!(xterm_256(232), (8, 8, 8));
        assert_eq!(xterm_256(255), (238, 238, 238));
    }

    /// The reader's declaration has to reach a value, or `[theme]` is a
    /// setting nothing reads and a reader who sets it is worse off than one
    /// who never found it.
    #[test]
    fn the_declared_background_selects_between_palettes() {
        assert_ne!(voice(Background::Dark), voice(Background::Light));
    }

    /// And the dark background carries the band on an attribute, which is
    /// the reason it is the one an undeclared reader gets: an attribute is
    /// what a reader with `NO_COLOR` set is still sent, and the light
    /// background has no way to give them one.
    #[test]
    fn the_dark_background_carries_the_band_on_an_attribute() {
        assert!(
            voice(Background::Dark).add_modifier.contains(Modifier::DIM),
            "the background most readers get is the one colour can be off on"
        );
    }

    /// The band is `bdi` scanning its own words at the reader, so it takes
    /// neither treatment that means something else: a slant is an author's
    /// emphasis and `markdown.rs`'s cyan is code, and an underline is
    /// somewhere to go.
    #[test]
    fn neither_background_answers_the_band_in_a_treatment_that_means_something_else() {
        for background in [Background::Dark, Background::Light] {
            let said = voice(background);
            assert!(
                !said
                    .add_modifier
                    .intersects(Modifier::ITALIC | Modifier::UNDERLINED),
                "{background:?} took a treatment that means something else"
            );
            assert_ne!(said.fg, CODE.fg, "{background:?} took prose's own colour");
        }
    }
}
