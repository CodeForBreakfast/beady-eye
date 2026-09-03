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
/// two apart, and a reader with `NO_COLOR` set is sent every attribute and no
/// colour — so the one place a tone stands alone is the one place it cannot
/// be a colour. Dim over the terminal's own foreground rather than over a
/// grey, because every mechanism a terminal has for dim moves a colour toward
/// the background, and colour 8 is already as near it as a readable slot
/// gets.
pub(crate) const VOICE: Style = Style::new().fg(Color::Reset).add_modifier(Modifier::DIM);

/// The page under the bead window's head: its facts, its prose and its
/// related rows. The rung directly under the terminal's default, so the few
/// things the window holds at the default read as emphasis rather than as the
/// page. It holds `QUIET`'s value and is a claim about a page of text rather
/// than about a row, so the forest's scale can move without the window
/// following it.
pub(crate) const PAGE: Style = Style::new().fg(Color::DarkGray);

/// `bdi`'s own sentence about the forest where nothing went wrong in it: the
/// hidden trees the filter is holding back, and a forest that was read and
/// held no work. Said at the terminal's own foreground because the thing it
/// contrasts with is `ATTENTION`.
pub(crate) const PLAIN: Style = Style::new().fg(Color::Reset);

/// The bead window's head — the glyph, id and title of the bead the reader
/// came for — at the terminal's own foreground, as that row is in the forest.
/// It holds `STRUCTURE`'s value and is a claim about the window's head.
pub(crate) const HEAD: Style = Style::new().fg(Color::Reset);

/// A code span or a code block: prose's own namespace.
pub(crate) const CODE: Style = Style::new().fg(Color::Cyan);

// ---- weights -----------------------------------------------------------

/// The row under the cursor, drawn so the eye finds it without reading it.
pub(crate) const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);

/// A window's own name, on its border.
pub(crate) const TITLE: Style = Style::new().add_modifier(Modifier::BOLD);

/// `bd show`'s section names, which stand out from a page of text: the
/// terminal's own foreground held under the page's tone, and a weight.
pub(crate) const SECTION: Style = Style::new().fg(Color::Reset).add_modifier(Modifier::BOLD);

/// A heading in prose. A weight and no colour, so it composes onto the tone
/// of the page it is drawn on.
pub(crate) const HEADING: Style = Style::new().add_modifier(Modifier::BOLD);

/// Prose's own emphasis, each a weight alone for the reason a heading is.
pub(crate) const EMPHASIS: Style = Style::new().add_modifier(Modifier::ITALIC);
pub(crate) const STRONG: Style = Style::new().add_modifier(Modifier::BOLD);
pub(crate) const LINK: Style = Style::new().add_modifier(Modifier::UNDERLINED);
