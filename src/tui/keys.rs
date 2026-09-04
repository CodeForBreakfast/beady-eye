//! Every key `bdi` answers, and the word for it a reader sees.
//!
//! The mapping, the key bindings view and the row under the tail are one
//! table read three ways, so a key is written down once and nothing on
//! screen can disagree with what pressing it does. Pure: no terminal, no
//! channel, nothing of the loop that reads the keystroke.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::view::{Action, Motion, Typing};

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
/// they are missing are being filtered. `Esc` sits below `q`: the bead view
/// names it in its own title, so nobody has to find it here.
pub(super) const BINDINGS: &[Binding] = &[
    Binding {
        keys: &[alone(KeyCode::Enter, "Enter")],
        action: Action::ShowBead,
        does: "show the selected bead, or focus its pane from the bead view",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('f'), "f")],
        action: Action::Focus,
        does: "focus the selected bead's pane",
        hint: None,
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
        keys: &[alone(KeyCode::Char('/'), "/")],
        action: Action::Search,
        does: "find part of a bead's id or title, wherever the forest draws it",
        hint: Some("find"),
    },
    Binding {
        keys: &[alone(KeyCode::Char('n'), "n")],
        action: Action::NextMatch,
        does: "go to the next bead matching the search",
        // No permanent word under the tail. It would cost a column on every
        // screen for a key that means nothing until a search has been made,
        // and `/ find` is already there saying searching exists.
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('N'), "N")],
        action: Action::PreviousMatch,
        does: "go to the one before it",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('q'), "q"), ctrl('c', "^C")],
        action: Action::Quit,
        does: "quit",
        hint: Some("quit"),
    },
    Binding {
        keys: &[alone(KeyCode::Esc, "Esc")],
        action: Action::Back,
        does: "go back to the forest from the bead view",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Tab, "Tab")],
        action: Action::NextRelated,
        does: "move to the next bead the shown bead names; Enter follows it",
        hint: None,
    },
    Binding {
        keys: &[ctrl('r', "^R")],
        action: Action::Refresh,
        does: "collect from the trackers again now",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('E'), "E")],
        action: Action::ExpandSubtree,
        does: "expand the selected node and everything under it",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('C'), "C")],
        action: Action::CollapseSubtree,
        does: "collapse the selected node and everything under it",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('D'), "D")],
        action: Action::RestoreDefault,
        does: "restore the default view",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('y'), "y")],
        action: Action::CopyId,
        does: "copy the selected bead's id to the clipboard",
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
        keys: &[ctrl('d', "^D"), alone(KeyCode::PageDown, "PgDn")],
        action: Action::Move(Motion::HalfScreenDown),
        does: "move down half a screen",
        hint: None,
    },
    Binding {
        keys: &[ctrl('u', "^U"), alone(KeyCode::PageUp, "PgUp")],
        action: Action::Move(Motion::HalfScreenUp),
        does: "move up half a screen",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Home, "Home"), alone(KeyCode::Char('g'), "g")],
        action: Action::Move(Motion::FirstRow),
        does: "move to the first row",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::End, "End"), alone(KeyCode::Char('G'), "G")],
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

/// What a keystroke does to the search prompt, or nothing where the prompt
/// has no use for it.
///
/// A mapping of its own beside the table rather than more rows in it. The
/// table is the keys that do something, and while the prompt is up almost
/// every key is a character of an id instead — so a row per letter would be a
/// key bindings screen nobody could read, listing keys that mean this only
/// here.
///
/// Nothing for a key held with control, which is what leaves `^C` to the
/// table: raw mode swallows it, and the key everyone reaches for to get out
/// of a program must not be inert because a prompt is up.
pub(super) fn typing(key: KeyEvent) -> Option<Typing> {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return None;
    }
    match key.code {
        KeyCode::Char(glyph) => Some(Typing::Character(glyph)),
        KeyCode::Backspace => Some(Typing::RubbedOut),
        KeyCode::Enter => Some(Typing::Sought),
        KeyCode::Esc => Some(Typing::Abandoned),
        _ => None,
    }
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
///
/// Four is what forty columns holds, so a key joining the row puts one out.
/// `Enter` went for `/`: pressing Enter on the thing under the cursor is what
/// a reader of any list does anyway, and pressing it here shows them what it
/// does. Nothing tells them `bdi` can be searched at all, so `/` is a key
/// they would otherwise never reach for — which is the same argument that
/// keeps `a` here.
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

#[cfg(test)]
pub(super) mod tests {
    //! The table's own tests, and the two helpers that name a keystroke.
    //!
    //! `key` and `control` build a `KeyEvent`, which is this module's
    //! vocabulary, so they live here and the loop tests in `tui/mod.rs`
    //! import them — the direction `tui/mod.rs` already depends in. What is
    //! shared is the naming of a keystroke; what a test then asserts about it
    //! stays with the test.

    use super::*;

    pub(in crate::tui) fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    pub(in crate::tui) fn control(code: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(code), KeyModifiers::CONTROL)
    }

    /// Every action there is.
    ///
    /// The match is what makes it every one rather than every one anybody
    /// remembered: an action or a motion added to the enums makes it
    /// non-exhaustive, and the compiler names this function until the list
    /// above it has grown too.
    fn every_action() -> Vec<Action> {
        let every = vec![
            Action::Move(Motion::PreviousRow),
            Action::Move(Motion::NextRow),
            Action::Move(Motion::HalfScreenUp),
            Action::Move(Motion::HalfScreenDown),
            Action::Move(Motion::FirstRow),
            Action::Move(Motion::LastRow),
            Action::CollapseOrParent,
            Action::ExpandOrChild,
            Action::ToggleFold,
            Action::ExpandSubtree,
            Action::CollapseSubtree,
            Action::RestoreDefault,
            Action::ToggleFilter,
            Action::Focus,
            Action::ShowBead,
            Action::NextRelated,
            Action::Back,
            Action::CopyId,
            Action::ShowBindings,
            Action::Search,
            Action::NextMatch,
            Action::PreviousMatch,
            Action::Refresh,
            Action::Quit,
        ];

        for action in &every {
            match action {
                Action::Move(motion) => match motion {
                    Motion::PreviousRow
                    | Motion::NextRow
                    | Motion::HalfScreenUp
                    | Motion::HalfScreenDown
                    | Motion::FirstRow
                    | Motion::LastRow => (),
                },
                Action::CollapseOrParent
                | Action::ExpandOrChild
                | Action::ToggleFold
                | Action::ExpandSubtree
                | Action::CollapseSubtree
                | Action::RestoreDefault
                | Action::ToggleFilter
                | Action::Focus
                | Action::ShowBead
                | Action::NextRelated
                | Action::Back
                | Action::CopyId
                | Action::ShowBindings
                | Action::Search
                | Action::NextMatch
                | Action::PreviousMatch
                | Action::Refresh
                | Action::Quit => (),
            }
        }

        every
    }

    /// An action no key reaches is one nobody can ask for, and it would have
    /// no line in the key bindings view either.
    ///
    /// `Forest::apply` already fails the build on an action added to the enum
    /// and matched nowhere. This is the other half: one that compiles
    /// everywhere and is still unreachable.
    #[test]
    fn every_action_has_a_key_that_asks_for_it() {
        for action in every_action() {
            assert!(
                BINDINGS.iter().any(|binding| binding.action == action),
                "{action:?} is bound to no key"
            );
        }
    }

    /// Graeme could not tell what `⏎` was, let alone press it. A key named
    /// with anything but the characters on a keyboard is that bug again.
    #[test]
    fn no_key_is_named_with_anything_a_keyboard_does_not_carry() {
        for binding in BINDINGS {
            for bound in binding.keys {
                assert!(!bound.named.is_empty(), "a key with no name");
                assert!(
                    bound
                        .named
                        .chars()
                        .all(|glyph| glyph.is_ascii_graphic() || glyph == ' '),
                    "{:?} is not a name anyone can press",
                    bound.named
                );
            }
        }
    }

    /// The row under the tail said `⏎` while the mapping said `Enter`, which
    /// is the whole of the bug. Both now come off the one table.
    #[test]
    fn the_row_under_the_tail_names_its_keys_as_the_mapping_does() {
        let row = key_row();

        for binding in BINDINGS {
            let Some(word) = binding.hint else { continue };
            let named = binding.keys.first().expect("a key").named;
            assert!(
                row.contains(&format!("{named} {word}")),
                "{named} {word} missing from {row:?}"
            );
        }
        assert!(row.contains("? keys"), "the way to the rest: {row:?}");
    }

    /// A binding added as a match arm rather than to the table would answer a
    /// key the view never mentions. Nothing is left that could do that.
    #[test]
    fn the_mapping_answers_no_key_the_table_does_not_name() {
        let named: Vec<&Key> = BINDINGS.iter().flat_map(|binding| binding.keys).collect();
        let swept = (' '..='~')
            .flat_map(|glyph| [key(KeyCode::Char(glyph)), control(glyph)])
            .chain([
                key(KeyCode::Up),
                key(KeyCode::Down),
                key(KeyCode::Left),
                key(KeyCode::Right),
                key(KeyCode::Enter),
                key(KeyCode::Tab),
                key(KeyCode::Esc),
                key(KeyCode::Backspace),
                key(KeyCode::Home),
                key(KeyCode::End),
                key(KeyCode::PageUp),
                key(KeyCode::PageDown),
            ]);

        for pressed in swept {
            let control = pressed.modifiers.contains(KeyModifiers::CONTROL);
            let expected = named
                .iter()
                .any(|bound| bound.code == pressed.code && (control || !bound.control));
            assert_eq!(
                action(pressed).is_some(),
                expected,
                "the table and the mapping disagree about {pressed:?}"
            );
        }
    }

    #[test]
    fn every_binding_reaches_the_action_it_names() {
        let bound = [
            (key(KeyCode::Char('j')), Action::Move(Motion::NextRow)),
            (key(KeyCode::Down), Action::Move(Motion::NextRow)),
            (key(KeyCode::Char('k')), Action::Move(Motion::PreviousRow)),
            (key(KeyCode::Up), Action::Move(Motion::PreviousRow)),
            (key(KeyCode::Char('h')), Action::CollapseOrParent),
            (key(KeyCode::Left), Action::CollapseOrParent),
            (key(KeyCode::Char('l')), Action::ExpandOrChild),
            (key(KeyCode::Right), Action::ExpandOrChild),
            (key(KeyCode::Char('g')), Action::Move(Motion::FirstRow)),
            (key(KeyCode::Home), Action::Move(Motion::FirstRow)),
            (key(KeyCode::Char('G')), Action::Move(Motion::LastRow)),
            (key(KeyCode::End), Action::Move(Motion::LastRow)),
            (control('d'), Action::Move(Motion::HalfScreenDown)),
            (control('u'), Action::Move(Motion::HalfScreenUp)),
            (key(KeyCode::PageDown), Action::Move(Motion::HalfScreenDown)),
            (key(KeyCode::PageUp), Action::Move(Motion::HalfScreenUp)),
            (key(KeyCode::Char(' ')), Action::ToggleFold),
            (key(KeyCode::Enter), Action::ShowBead),
            (key(KeyCode::Char('f')), Action::Focus),
            (key(KeyCode::Esc), Action::Back),
            (key(KeyCode::Char('y')), Action::CopyId),
            (key(KeyCode::Char('a')), Action::ToggleFilter),
            (control('r'), Action::Refresh),
            (key(KeyCode::Char('/')), Action::Search),
            (key(KeyCode::Char('n')), Action::NextMatch),
            // `N` is here because nothing presses it. `n` is pressed by a
            // screen test that reads the bead and the count it lands on, so
            // its wiring is asserted by what it does; `N` is reached from no
            // test at all, and `every_action_has_a_key_that_asks_for_it` is
            // reachability rather than direction — satisfied by any bijection
            // over the actions. So this line is where `N`'s wiring is pinned
            // or it is pinned nowhere.
            (key(KeyCode::Char('N')), Action::PreviousMatch),
            (key(KeyCode::Char('?')), Action::ShowBindings),
            (key(KeyCode::Char('q')), Action::Quit),
            (control('c'), Action::Quit),
        ];

        for (pressed, expected) in bound {
            assert_eq!(action(pressed), Some(expected), "for {pressed:?}");
        }
    }

    /// The prompt takes the keys, because a reader typing an id into it is
    /// pressing letters that mean something else everywhere on this screen.
    #[test]
    fn every_key_of_an_id_is_a_character_of_it_while_the_prompt_is_up() {
        for glyph in ('!'..='~').chain([' ']) {
            assert_eq!(
                typing(key(KeyCode::Char(glyph))),
                Some(Typing::Character(glyph)),
                "{glyph:?} is not a character of an id"
            );
        }
    }

    /// The four keys that are not a character of an id, and the one that is
    /// not the prompt's at all: `^C` leaves `bdi` from the prompt as it does
    /// from everywhere else, because raw mode swallows it and the table's
    /// alias is the whole of what answers it.
    #[test]
    fn the_prompt_answers_the_keys_that_work_a_prompt_and_leaves_control_alone() {
        assert_eq!(typing(key(KeyCode::Backspace)), Some(Typing::RubbedOut));
        assert_eq!(typing(key(KeyCode::Enter)), Some(Typing::Sought));
        assert_eq!(typing(key(KeyCode::Esc)), Some(Typing::Abandoned));
        assert_eq!(typing(key(KeyCode::Up)), None, "a motion is not typing");

        assert_eq!(typing(control('c')), None, "^C is not a character of an id");
        assert_eq!(action(control('c')), Some(Action::Quit));
    }

    /// The letters that carry a binding only under control carry none on
    /// their own, and a key nothing is bound to asks for nothing.
    #[test]
    fn a_key_bound_to_nothing_asks_for_nothing() {
        for pressed in [
            key(KeyCode::Char('d')),
            key(KeyCode::Char('u')),
            key(KeyCode::Char('r')),
            key(KeyCode::Char('c')),
            key(KeyCode::Char('z')),
        ] {
            assert_eq!(action(pressed), None, "for {pressed:?}");
        }
    }
}
