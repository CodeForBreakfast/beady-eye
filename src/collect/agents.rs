//! What the sessions on this machine answer about their panes, whichever
//! program answers it.
//!
//! Four questions, and `bdi` asks a session nothing else. How one reaches a
//! session — herdr's command line today, another multiplexer some day — is an
//! adapter's business, and this is the line it stays behind.
//!
//! All four are answered in full here. The two that must not hold up the
//! thread reading keys are made asynchronous in `collect::panes`, once, over
//! whichever provider a run was given.

use crate::collect::run::RunFailure;
use crate::model::types::{Pane, PaneKey};

/// The agent provider: the sessions on this machine, the panes each holds,
/// and the one thing `bdi` does to one of them.
///
/// `Send + Sync` because a run holds one of these and asks it from two
/// threads — the collection asks `sessions` and `list` while the tail asks
/// `read`.
pub trait Agents: Send + Sync {
    /// What to call this provider wherever `bdi` says which one it read.
    fn name(&self) -> &'static str;

    /// Every session the provider is running, by name. A session that is
    /// not running holds no pane and is not here.
    fn sessions(&self) -> Result<Vec<String>, RunFailure>;

    /// Every pane one session holds, each with where it is working and what
    /// it is showing.
    fn list(&self, session: &str) -> Result<Vec<Pane>, RunFailure>;

    /// The last `lines` rows one pane drew, in the styling it drew them in.
    fn read(&self, pane: &PaneKey, lines: u16) -> Result<Vec<String>, RunFailure>;

    /// Bring a pane to the front. The only write `bdi` performs.
    fn focus(&self, pane: &PaneKey) -> Result<(), RunFailure>;
}

/// A provider shared between the collection that lists and the tail that
/// reads, each on its own thread, is held as one of these — so the seam
/// carries through the sharing rather than stopping at it.
impl<A: Agents + ?Sized> Agents for std::sync::Arc<A> {
    fn name(&self) -> &'static str {
        (**self).name()
    }

    fn sessions(&self) -> Result<Vec<String>, RunFailure> {
        (**self).sessions()
    }

    fn list(&self, session: &str) -> Result<Vec<Pane>, RunFailure> {
        (**self).list(session)
    }

    fn read(&self, pane: &PaneKey, lines: u16) -> Result<Vec<String>, RunFailure> {
        (**self).read(pane, lines)
    }

    fn focus(&self, pane: &PaneKey) -> Result<(), RunFailure> {
        (**self).focus(pane)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{pane, Fake, THE_FAKE};
    use super::*;
    use crate::model::types::testing::{key, A_SESSION};
    use crate::model::types::PaneStatus;
    use pretty_assertions::assert_eq;
    use std::sync::Arc;

    /// A run holds its provider as an `Arc` and asks it through that, so every
    /// question this crate ever puts to a provider goes through the forwarding
    /// impl rather than to the adapter directly. Nothing else reaches it: the
    /// collection and the tail are handed the shared one and no test of either
    /// would notice a question that stopped at the `Arc`.
    #[test]
    fn a_shared_provider_answers_as_the_one_it_holds() {
        let alone = Fake::holding(vec![pane("w:p1", "/srv/work", PaneStatus::Idle)])
            .showing("w:p1", ["what the pane drew"]);
        let shared: Arc<dyn Agents> = Arc::new(alone);

        assert_eq!(shared.name(), THE_FAKE);
        assert_eq!(
            shared.sessions().expect("the provider runs one session"),
            [A_SESSION]
        );
        assert_eq!(
            shared.list(A_SESSION).expect("the provider holds one pane")[0].pane_id,
            "w:p1"
        );
        assert_eq!(
            shared.read(&key("w:p1"), 1).expect("the pane was staged"),
            ["what the pane drew"]
        );
        assert_eq!(shared.focus(&key("w:p1")), Ok(()));
    }

    /// A focus that failed has to come back through the share as a failure.
    ///
    /// The success above cannot say so: `Ok(())` is what a forwarding impl
    /// that answered for itself and never asked would give, so the only
    /// reading that tells the two apart is a provider whose focus fails.
    #[test]
    fn a_focus_the_provider_refused_is_refused_through_the_share() {
        let refusing = Fake::holding(Vec::new()).unfocusable(RunFailure {
            kind: crate::collect::run::FailureKind::Gone,
            program: THE_FAKE.to_string(),
            detail: "the pane is not there".to_string(),
            unreadable: None,
        });
        let shared: Arc<dyn Agents> = Arc::new(refusing);

        assert_eq!(
            shared.focus(&key("w:gone")).map_err(|failure| failure.kind),
            Err(crate::collect::run::FailureKind::Gone)
        );
    }
}

/// A provider that answers from what a test staged, and counts what it was
/// asked. Shared by the tests inside the crate and the ones under `tests/`,
/// which is why it sits behind the feature rather than `cfg(test)`.
#[cfg(feature = "testing")]
pub mod testing {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::model::types::testing::A_SESSION;
    use crate::model::types::PaneStatus;

    /// What a fake provider calls itself, so a test can tell a snapshot built
    /// over one from a snapshot built over herdr.
    pub const THE_FAKE: &str = "a fake provider";

    /// One of the four questions, as a fake records being asked it.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Asked {
        Sessions,
        List { session: String },
        Read { pane: PaneKey, lines: u16 },
        Focus { pane: PaneKey },
    }

    /// One pane, as a test says it rather than as herdr spells it, in the
    /// session a test's panes are in unless it says otherwise.
    ///
    /// The whole of `Pane` a provider has to answer with: a pane that has
    /// identified itself adds to this, and one that has not is this. It is
    /// the only place inside the crate that builds a `Pane` from parts, so a
    /// field added to it is added here and nowhere else.
    pub fn pane(id: &str, cwd: &str, status: PaneStatus) -> Pane {
        Pane::answered(
            A_SESSION.to_string(),
            id.to_string(),
            PathBuf::from(cwd),
            status,
        )
    }

    /// The same pane, in another session.
    pub fn in_session(mut pane: Pane, session: &str) -> Pane {
        pane.session = session.to_string();
        pane
    }

    /// The same pane, saying which bead it is working.
    pub fn named(mut pane: Pane, bead: &str) -> Pane {
        pane.display_agent = Some(bead.to_string());
        pane
    }

    /// The same pane, saying what it is doing.
    pub fn titled(mut pane: Pane, title: &str) -> Pane {
        pane.title = Some(title.to_string());
        pane
    }

    /// A provider answering each question from what was staged.
    ///
    /// The sessions it runs are the ones its panes are in, and the one a
    /// test's panes are in unless it says otherwise is always among them —
    /// so a provider holding no pane still runs one session and answers for
    /// it, which is a provider that answered and not one that did not.
    #[derive(Default)]
    pub struct Fake {
        sessions: Option<RunFailure>,
        panes: Vec<Pane>,
        unanswering: BTreeMap<String, RunFailure>,
        reads: BTreeMap<PaneKey, Result<Vec<String>, RunFailure>>,
        focus: Option<RunFailure>,
        asked: Mutex<Vec<Asked>>,
    }

    impl Fake {
        /// A provider holding `panes`, with nothing staged to read and every
        /// focus succeeding.
        pub fn holding(panes: Vec<Pane>) -> Self {
            Self {
                panes,
                ..Self::default()
            }
        }

        /// A provider that fails to say which sessions it runs, rather than
        /// answering — the two states a run is in when it has no panes,
        /// told apart by the kind.
        pub fn unlistable(failure: RunFailure) -> Self {
            Self {
                sessions: Some(failure),
                ..Self::default()
            }
        }

        /// This provider, with `session` among the ones it runs and failing
        /// to list its panes rather than answering.
        pub fn not_answering_for(mut self, session: &str, failure: RunFailure) -> Self {
            self.unanswering.insert(session.to_string(), failure);
            self
        }

        /// Reading `pane` answers with `lines`.
        pub fn showing<'a>(mut self, pane: &str, lines: impl IntoIterator<Item = &'a str>) -> Self {
            self.reads.insert(
                crate::model::types::testing::key(pane),
                Ok(lines.into_iter().map(str::to_string).collect()),
            );
            self
        }

        /// Every focus fails rather than succeeding.
        pub fn unfocusable(mut self, failure: RunFailure) -> Self {
            self.focus = Some(failure);
            self
        }

        /// Every question this provider has been asked, in order.
        pub fn asked(&self) -> Vec<Asked> {
            self.asked
                .lock()
                .expect("no test panics holding this")
                .clone()
        }

        fn note(&self, question: Asked) {
            self.asked
                .lock()
                .expect("no test panics holding this")
                .push(question);
        }
    }

    /// A read nobody staged, said the way a provider says a pane it has never
    /// heard of.
    fn no_such_pane(pane: &PaneKey) -> RunFailure {
        RunFailure {
            kind: crate::collect::run::FailureKind::Gone,
            program: THE_FAKE.to_string(),
            detail: format!("no test staged a read of {} in {}", pane.id, pane.session),
            unreadable: None,
        }
    }

    impl Agents for Fake {
        fn name(&self) -> &'static str {
            THE_FAKE
        }

        fn sessions(&self) -> Result<Vec<String>, RunFailure> {
            self.note(Asked::Sessions);
            if let Some(failure) = &self.sessions {
                return Err(failure.clone());
            }
            let mut sessions = vec![A_SESSION.to_string()];
            for session in self
                .panes
                .iter()
                .map(|pane| &pane.session)
                .chain(self.unanswering.keys())
            {
                if !sessions.contains(session) {
                    sessions.push(session.clone());
                }
            }
            Ok(sessions)
        }

        fn list(&self, session: &str) -> Result<Vec<Pane>, RunFailure> {
            self.note(Asked::List {
                session: session.to_string(),
            });
            if let Some(failure) = self.unanswering.get(session) {
                return Err(failure.clone());
            }
            Ok(self
                .panes
                .iter()
                .filter(|pane| pane.session == session)
                .cloned()
                .collect())
        }

        fn read(&self, pane: &PaneKey, lines: u16) -> Result<Vec<String>, RunFailure> {
            self.note(Asked::Read {
                pane: pane.clone(),
                lines,
            });
            self.reads
                .get(pane)
                .cloned()
                .unwrap_or_else(|| Err(no_such_pane(pane)))
        }

        fn focus(&self, pane: &PaneKey) -> Result<(), RunFailure> {
            self.note(Asked::Focus { pane: pane.clone() });
            self.focus.clone().map_or(Ok(()), Err)
        }
    }
}
