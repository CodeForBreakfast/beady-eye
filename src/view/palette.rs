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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::Background;

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
