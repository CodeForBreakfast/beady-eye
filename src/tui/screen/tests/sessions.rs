//! A seat in another herdr session is read and focused in that session.
//!
//! A pane id is minted per session, so the id a bead names is a pane only
//! together with the session holding it. A read that named no session would
//! read whichever session `bdi` sits in, and a focus would move somebody
//! else's terminal.

use super::*;

const ANOTHER_SESSION: &str = "kadath";

/// The provider, remembering each pane it was asked about by its whole key.
#[derive(Clone, Default)]
struct Keyed {
    reads: Arc<Mutex<Vec<PaneKey>>>,
    focuses: Arc<Mutex<Vec<PaneKey>>>,
}

impl Panes for Keyed {
    fn read(&self, pane: &PaneKey, _lines: u16) {
        self.reads
            .lock()
            .expect("no test panics holding this")
            .push(pane.clone());
    }

    fn focus(&self, pane: &PaneKey) {
        self.focuses
            .lock()
            .expect("no test panics holding this")
            .push(pane.clone());
    }
}

/// A staffed grove whose every seat is in another session, on a screen
/// asking `panes`.
fn seated_in_another_session(panes: &Keyed) -> Shown {
    let mut snapshot = a_staffed_grove(6);
    for tree in &mut snapshot.collected {
        for node in &mut Arc::make_mut(tree).beads {
            if let Some(agent) = &mut node.agent {
                agent.pane.session = ANOTHER_SESSION.to_string();
            }
        }
    }
    snapshot.trees = snapshot.collected.clone();
    Shown::of(
        snapshot,
        Box::new(panes.clone()),
        Box::new(io::sink()),
        drawing(),
        nothing_said(),
        an_instant(),
    )
}

fn the_selected_seat() -> PaneKey {
    PaneKey {
        session: ANOTHER_SESSION.to_string(),
        id: A_SELECTED_PANE.to_string(),
    }
}

#[test]
fn a_selected_seat_in_another_session_is_read_from_that_session() {
    let panes = Keyed::default();

    seated_in_another_session(&panes);

    assert_eq!(
        *panes.reads.lock().expect("no test panics holding this"),
        [the_selected_seat()]
    );
}

#[test]
fn a_focus_on_a_seat_in_another_session_is_asked_of_that_session() {
    let panes = Keyed::default();
    let mut shown = seated_in_another_session(&panes);

    shown.apply(Action::Focus);

    assert_eq!(
        *panes.focuses.lock().expect("no test panics holding this"),
        [the_selected_seat()]
    );
}
