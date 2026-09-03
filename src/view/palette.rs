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

/// A row an agent is on: the terminal's own foreground, not a brighter
/// colour, because a theme's default is already the brightest thing on its
/// page and nothing can sit above it. The scale is shifted down from here
/// rather than extended up.
pub(crate) const TIER_STAFFED: Style = Style::new().fg(Color::Reset);

/// Nobody on it and still going. One rung below the terminal's default, at
/// the theme's colour 8, which every theme sets and few rows on the page
/// otherwise use. Named rather than literal so it follows the reader's
/// terminal, not `bd`'s palette.
pub(crate) const TIER_OPEN: Style = Style::new().fg(Color::DarkGray);

/// Finished, nobody on it: the grey `bd` dims a closed row to. It holds
/// `STATUS_DEFERRED`'s value and is a claim about the scale rather than about
/// a status, so moving the scale leaves `bd`'s own colour where it is.
pub(crate) const TIER_FINISHED: Style = Style::new().fg(Color::Rgb(108, 118, 128));

// ---- chrome, and content that is not a row -----------------------------

/// Box-drawing and fold arrows: how the tree is shaped rather than how a bead
/// is going, so it is held at the terminal's default while the row around it
/// dims or brightens.
pub(crate) const STRUCTURE: Style = Style::new().fg(Color::Reset);

/// Metadata, chrome, an affordance, a rule.
pub(crate) const QUIET: Style = Style::new().fg(Color::DarkGray);

/// In the tail band only: these are `bdi`'s words and not the pane's.
/// `design.md`'s account of the band makes this the whole of what tells the
/// two apart, so it holds `QUIET`'s value and is not it.
pub(crate) const VOICE: Style = Style::new().fg(Color::DarkGray);

/// The page under the bead window's head: its facts, its prose and its
/// related rows. The rung directly under the terminal's default, which is
/// where the forest draws a row nobody is on, so the few things the window
/// holds at the default read as emphasis rather than as the page. It holds
/// `TIER_OPEN`'s value and is a claim about a page of text rather than about
/// the scale.
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
