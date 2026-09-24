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
/// `control` is matched exactly: `c` and `^C` are two keys, and a letter
/// bound on its own does not answer for the control key over it. Any other
/// modifier held is ignored, which is what the arrows and the letters have
/// always done.
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
/// they are missing are being filtered. `Esc` sits below `q`: the bead
/// window's own row under the tail names it, so nobody has to find it here.
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
        keys: &[alone(KeyCode::Char('s'), "s")],
        action: Action::CycleSpine,
        does: "cycle which copy of a bead opens, under the selected node",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('S'), "S")],
        action: Action::CycleSpineForest,
        does: "cycle which copy of a bead opens, across the whole forest",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('F'), "F")],
        action: Action::FocusForest,
        does: "draw the selected bead as the only root, or put the forest back",
        hint: None,
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
        keys: &[alone(KeyCode::Char('e'), "e")],
        action: Action::ExpandSubtree,
        does: "expand the selected node and everything under it",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('E'), "E")],
        action: Action::ExpandForest,
        does: "expand the whole forest",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('c'), "c")],
        action: Action::CollapseSubtree,
        does: "collapse the selected node and everything under it",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('C'), "C")],
        action: Action::CollapseForest,
        does: "collapse the whole forest",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('d'), "d")],
        action: Action::RestoreSubtree,
        does: "restore the default folds under the selected node",
        hint: None,
    },
    Binding {
        keys: &[alone(KeyCode::Char('D'), "D")],
        action: Action::RestoreDefault,
        does: "restore the default folds across the whole forest",
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
                .any(|bound| bound.code == key.code && bound.control == control)
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

/// The bead window's row: what each key does there, in the order the row
/// says them.
///
/// The words and the order are written here rather than read off `BINDINGS`,
/// whose order is least-guessable-first for the `?` window and whose words
/// are the forest's. The keys are still named off `BINDINGS`, so the row
/// cannot call a key something the mapping does not answer to.
///
/// The order ranks them, because it is also the order the row gives them up
/// in. `Esc` is first: a reader who cannot see how to leave is stuck in a
/// view they may have opened by accident. `?` is second because it is the one
/// key that reaches every key this row had to leave off, so a row down to two
/// has still lost the reader nothing. The two left go to what a reader of
/// this window cannot work out for themselves: nothing on the page says the
/// beads it names can be stepped through, and taking the id away is what a
/// reader opens a bead to do.
///
/// `Enter` is off the row for the reason `key_row` already gives for taking
/// it off the forest's: pressing Enter on the thing under the cursor is what
/// a reader of any list does anyway, and `Tab related` is what puts a thing
/// under the cursor. The motion keys are off it for the reason the arrows
/// are, and how far down the bead the reader has got is the border title's to
/// say.
const IN_BEAD: &[(Action, &str)] = &[
    (Action::Back, "back"),
    (Action::ShowBindings, "keys"),
    (Action::NextRelated, "related"),
    (Action::CopyId, "id"),
];

/// The row under the tail while the bead window is up, and every shorter form
/// of it, fullest first.
///
/// The window holds four fifths of the screen, so a reader who cannot see
/// this row has nothing left to look at: the row gives up a key at a time
/// from the end rather than going whole, and `Esc back` is the last of them
/// to go. The forest's row hands the foot one form and still goes whole or
/// not at all, which is where that rule was written and is still right there
/// — a reader in the forest is held nowhere and can look.
pub(super) fn bead_key_rows() -> Vec<String> {
    let named: Vec<String> = IN_BEAD
        .iter()
        .filter_map(|(action, word)| {
            let named = BINDINGS
                .iter()
                .find(|binding| binding.action == *action)?
                .keys
                .first()?
                .named;
            Some(format!("{named} {word}"))
        })
        .collect();

    (1..=named.len())
        .rev()
        .map(|kept| named[..kept].join("   "))
        .collect()
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
            Action::RestoreSubtree,
            Action::ExpandForest,
            Action::CollapseForest,
            Action::RestoreDefault,
            Action::CycleSpine,
            Action::CycleSpineForest,
            Action::ToggleFilter,
            Action::Focus,
            Action::FocusForest,
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
                | Action::RestoreSubtree
                | Action::ExpandForest
                | Action::CollapseForest
                | Action::RestoreDefault
                | Action::CycleSpine
                | Action::CycleSpineForest
                | Action::ToggleFilter
                | Action::Focus
                | Action::FocusForest
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

    /// The bead window's row in full, which is the first form the foot is
    /// handed.
    fn whole_row() -> String {
        bead_key_rows().remove(0)
    }

    /// Forty columns is what the narrowest supported terminal holds, and a
    /// row wider than that would start every screen already giving keys up.
    #[test]
    fn the_bead_windows_row_fits_forty_columns() {
        let row = whole_row();

        assert!(
            row.chars().count() <= 40,
            "{} columns: {row:?}",
            row.chars().count()
        );
    }

    /// `Esc` first, because a reader who cannot see how to leave is stuck in
    /// a view they may have opened by accident.
    #[test]
    fn the_bead_windows_row_says_the_way_out_first() {
        let row = whole_row();

        assert!(row.starts_with("Esc back"), "{row:?}");
    }

    /// Four keys, and `?` among them: the row is full, so what is not on it
    /// is reachable from it.
    #[test]
    fn the_bead_windows_row_is_four_keys_and_one_of_them_is_the_way_to_the_rest() {
        let row = whole_row();

        assert_eq!(row.split("   ").count(), 4, "{row:?}");
        assert!(row.contains("? keys"), "the way to the rest: {row:?}");
    }

    /// A word in the row and no key to press for it is a row that names
    /// nothing, and `bead_key_rows` drops such an entry rather than saying
    /// so. This is what says none was dropped.
    #[test]
    fn every_key_the_bead_windows_row_names_is_one_the_mapping_answers() {
        let row = whole_row();

        for (action, word) in IN_BEAD {
            let binding = BINDINGS
                .iter()
                .find(|binding| binding.action == *action)
                .unwrap_or_else(|| panic!("{action:?} is bound to no key"));
            let named = binding.keys.first().expect("a key").named;
            assert!(
                row.contains(&format!("{named} {word}")),
                "{named} {word} missing from {row:?}"
            );
        }
    }

    /// Each form is the one before it with its last key taken off, so a
    /// narrowing row loses keys rather than gaining a different shape, and
    /// what is left at the end is the way out.
    #[test]
    fn the_bead_windows_row_gives_up_its_keys_from_the_end() {
        let forms = bead_key_rows();

        assert_eq!(forms.len(), IN_BEAD.len(), "{forms:?}");
        for (shorter, fuller) in forms.iter().skip(1).zip(&forms) {
            assert!(fuller.starts_with(shorter.as_str()), "{forms:?}");
            assert_eq!(
                shorter.split("   ").count() + 1,
                fuller.split("   ").count(),
                "{forms:?}"
            );
        }
        assert_eq!(forms.last().expect("a form of the row"), "Esc back");
    }

    /// `?` reaches every key the row had to leave off, so a row with room for
    /// two keys says it rather than one of the two things this window does.
    #[test]
    fn the_bead_windows_row_keeps_the_way_to_the_rest_past_what_the_window_does() {
        let forms = bead_key_rows();

        assert_eq!(
            forms.iter().rev().nth(1).expect("a form naming two keys"),
            "Esc back   ? keys"
        );
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
                .any(|bound| bound.code == pressed.code && bound.control == control);
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
            (key(KeyCode::Char('s')), Action::CycleSpine),
            (key(KeyCode::Char('S')), Action::CycleSpineForest),
            (key(KeyCode::Char('F')), Action::FocusForest),
            (key(KeyCode::Tab), Action::NextRelated),
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
            (key(KeyCode::Char('e')), Action::ExpandSubtree),
            (key(KeyCode::Char('E')), Action::ExpandForest),
            (key(KeyCode::Char('c')), Action::CollapseSubtree),
            (key(KeyCode::Char('C')), Action::CollapseForest),
            (key(KeyCode::Char('d')), Action::RestoreSubtree),
            (key(KeyCode::Char('D')), Action::RestoreDefault),
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
            key(KeyCode::Char('u')),
            key(KeyCode::Char('r')),
            key(KeyCode::Char('z')),
        ] {
            assert_eq!(action(pressed), None, "for {pressed:?}");
        }
    }

    /// A letter bound on its own and under control is two keys, and the
    /// control one does not answer for the letter.
    #[test]
    fn a_control_key_does_not_answer_for_the_letter_under_it() {
        assert_ne!(action(control('c')), action(key(KeyCode::Char('c'))));
        assert_ne!(action(control('d')), action(key(KeyCode::Char('d'))));
    }
}
