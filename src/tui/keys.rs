//! Every key `bdi` answers, and the word for it a reader sees.
//!
//! The mapping, the key bindings view and the row under the tail are one
//! table read three ways, so a key is written down once and nothing on
//! screen can disagree with what pressing it does. Pure: no terminal, no
//! channel, nothing of the loop that reads the keystroke.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::view::{Action, Motion};

/// One key a reader can press, and the word for it they can read.
///
/// `control` is a requirement and not an exclusion: a key that does not ask
/// for it answers whatever modifiers are held, which is what the arrows and
/// the letters have always done.
pub(super) struct Key {
    pub(super) code: KeyCode,
    pub(super) control: bool,
    pub(super) named: &'static str,
}

const fn alone(code: KeyCode, named: &'static str) -> Key {
    Key {
        code,
        control: false,
        named,
    }
}

const fn ctrl(code: char, named: &'static str) -> Key {
    Key {
        code: KeyCode::Char(code),
        control: true,
        named,
    }
}

/// One thing the view does: the keys that ask for it, and what to call it.
pub(super) struct Binding {
    pub(super) keys: &'static [Key],
    pub(super) action: Action,
    /// What pressing it does, for the key bindings view.
    pub(super) does: &'static str,
    /// Its word in the row under the tail, for the few that earn a permanent
    /// line there.
    pub(super) hint: Option<&'static str>,
}

/// Every binding there is.
///
/// The mapping and the key bindings view are this table read two ways, so a
/// key is written down once and nothing on screen can disagree with what
/// pressing it does. `^C` is an alias for `q`: raw mode swallows it, and the
/// key everyone reaches for must not be inert.
///
/// The order is least guessable first, because a screen too short for the
/// whole table shows the top of it. A reader who cannot see the arrows will
/// press one anyway; one who cannot see `a` will not work out that the trees
/// they are missing are being filtered.
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        keys: &[alone(KeyCode::Enter, "Enter")],
        action: Action::Focus,
        does: "focus the selected bead's pane in herdr",
        hint: Some("focus"),
    },
    Binding {
        keys: &[alone(KeyCode::Char(' '), "Space")],
        action: Action::ToggleFold,
        does: "fold or unfold the selected node",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('a'), "a")],
        action: Action::ToggleFilter,
        does: "show every tree, not only those with a live agent",
        hint: Some("all"),
    },
    Binding {
        keys: &[alone(KeyCode::Char('?'), "?")],
        action: Action::ShowBindings,
        does: "show these key bindings",
        hint: Some("keys"),
    },
    Binding {
        keys: &[alone(KeyCode::Char('q'), "q"), ctrl('c', "^C")],
        action: Action::Quit,
        does: "quit",
        hint: Some("quit"),
    },
    Binding {
        keys: &[ctrl('r', "^R")],
        action: Action::Refresh,
        does: "collect from the trackers again now",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('E'), "E")],
        action: Action::ExpandAll,
        does: "expand every node",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('C'), "C")],
        action: Action::CollapseAll,
        does: "collapse every node",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('D'), "D")],
        action: Action::RestoreDefault,
        does: "restore the default view",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Down, "Down"), alone(KeyCode::Char('j'), "j")],
        action: Action::Move(Motion::NextRow),
        does: "move down one row",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Up, "Up"), alone(KeyCode::Char('k'), "k")],
        action: Action::Move(Motion::PreviousRow),
        does: "move up one row",
        hint: None,
    },
    Binding {
        keys: &[
            alone(KeyCode::Right, "Right"),
            alone(KeyCode::Char('l'), "l"),
        ],
        action: Action::ExpandOrChild,
        does: "expand, or move to the first child when it is already expanded",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Left, "Left"), alone(KeyCode::Char('h'), "h")],
        action: Action::CollapseOrParent,
        does: "collapse, or move to the parent when it is already collapsed",
        hint: None,
    },
    Binding {
        keys: &[ctrl('d', "^D")],
        action: Action::Move(Motion::HalfScreenDown),
        does: "move down half a screen",
        hint: None,
    },
    Binding {
        keys: &[ctrl('u', "^U")],
        action: Action::Move(Motion::HalfScreenUp),
        does: "move up half a screen",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('g'), "g")],
        action: Action::Move(Motion::FirstRow),
        does: "move to the first row",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('G'), "G")],
        action: Action::Move(Motion::LastRow),
        does: "move to the last row",
        hint: None,
    },
];

/// The action a key asks for, or nothing where it is bound to none.
pub(super) fn action(key: KeyEvent) -> Option<Action> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);

    BINDINGS
        .iter()
        .find(|binding| {
            binding
                .keys
                .iter()
                .any(|bound| bound.code == key.code && (control || !bound.control))
        })
        .map(|binding| binding.action)
}

/// Every binding named for a reader: the keys to press, and what pressing
/// them does.
pub(super) fn bindings() -> Vec<(String, &'static str)> {
    BINDINGS
        .iter()
        .map(|binding| {
            (
                binding
                    .keys
                    .iter()
                    .map(|key| key.named)
                    .collect::<Vec<_>>()
                    .join(", "),
                binding.does,
            )
        })
        .collect()
}

/// The row under the tail: the handful of bindings worth a permanent line,
/// each named by the first key that reaches it.
///
/// Naming a key costs more columns than a glyph did, and a row that outgrew a
/// forty-column terminal would lose its last words — which is `q quit`. So the
/// row keeps the keys a reader reaches for and leaves the rest to `?`, which
/// is the one it gains.
pub(super) fn key_row() -> String {
    BINDINGS
        .iter()
        .filter_map(|binding| {
            let word = binding.hint?;
            Some(format!("{} {word}", binding.keys.first()?.named))
        })
        .collect::<Vec<_>>()
        .join("   ")
}
