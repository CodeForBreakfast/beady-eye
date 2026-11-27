//! A web page's address: what it asks the forest to show, and the address
//! of every page a link on it goes to.
//!
//! The forest is at `/` and one bead is at `/<project>/<id>`, each part
//! percent-encoded. The folds a reader has pointed ride in the query, each
//! named by what its line is known by and which way it points, so a page
//! keeps nothing between requests: the forest a request asks for is
//! flattened afresh from the snapshot and the folds are pointed on it. A link
//! from a line names the copy of the bead the line draws too, so where the
//! forest draws a bead more than once the page lands on that copy.
//!
//! What else a key changes rides in the query too: the list drawn in place
//! of the trees, the bead the forest is rooted at, and what was searched
//! for. Each is put on the forest by the forest's own action, before the
//! folds, so the page draws what the terminal draws after the same keys.

use std::collections::{BTreeSet, HashMap};
use std::iter;

use base64::prelude::{Engine as _, BASE64_URL_SAFE_NO_PAD};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::model::join::BeadKey;
use crate::model::snapshot::Snapshot;
use crate::view::forest::{flatten, listed, only_ready, view_of, Forest, Landed, List};
use crate::view::lines::{Content, Line, Place};
use crate::view::row::Layout;
use crate::view::{Action, Write};

/// The query keys a fold pointed open and a fold pointed shut are written
/// under.
const OPEN: &str = "open";
const SHUT: &str = "shut";

/// The query key naming which copy of a bead drawn more than once the page
/// lands on. A step through a search's matches steps from this copy.
const AT: &str = "at";

/// The query keys naming the view drawn in place of the trees, and whether
/// only ready beads are listed, which `READY` says with the value `ONLY`.
const VIEW: &str = "view";
const READY: &str = "ready";
const ONLY: &str = "1";

/// The value `READY` says the trees with, on a page whose start is a list.
const TREES: &str = "0";

/// The query keys naming the list of the beads to check and the list of the
/// beads left to do, each said with the value `ONLY`, and the one project a
/// list keeps to.
const CHECK: &str = "check";
const LEFT: &str = "left";
const PROJECT: &str = "project";

/// The query key naming the bead the forest is drawn with as its only root,
/// and with no bead, every tree, on a page whose start is rooted.
const ROOT: &str = "root";

/// The query keys holding what was searched for, and which way a step
/// through its matches goes. A search's form names what is typed into it
/// by `FIND`.
pub const FIND: &str = "find";
const STEP: &str = "step";
const NEXT: &str = "next";
const BACK: &str = "back";

/// The anchor on the line a bead's address shows the bead on, which every
/// link to a bead ends in, so the page it opens is scrolled to the bead.
pub const SHOWN: &str = "shown";

/// What a page shows where its address does not say: the list and the beads
/// `bdi web` was started on, as the terminal view starts on them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Start {
    /// The list drawn in place of the trees, where one is.
    pub list: Option<List>,
    /// The beads the forest is drawn with as its only roots.
    pub roots: Vec<BeadKey>,
}

/// What the forest is drawn with as its only roots.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Root {
    /// The beads the page starts on, or every tree where it starts on none.
    #[default]
    AsStarted,
    /// Every tree.
    Whole,
    /// The bead at this place.
    At(Place),
}

/// What a request asks the page to show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Asked {
    /// The bead whose address was asked for, where one was.
    pub bead: Option<BeadKey>,
    /// The folds the reader has pointed, each named once.
    pub folds: Vec<Fold>,
    /// The copy of the bead the reader followed, where the forest draws it
    /// more than once.
    pub place: Option<Place>,
    /// The list drawn in place of the trees, where one is.
    pub list: Option<List>,
    /// Whether `list` is the start's, the address naming none.
    pub list_from_start: bool,
    /// What the forest is drawn with as its only roots.
    pub root: Root,
    /// What was searched for, where a page asks for a search's answer
    /// rather than for a bead.
    pub find: Option<Find>,
    /// What the page shows where its address does not say.
    pub start: Start,
}

/// A search a page asks for: the text, and whether the page steps on from
/// the match it was on, forward or back, rather than searching afresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Find {
    pub text: String,
    pub step: Option<bool>,
}

/// One fold the reader has pointed: what its line is known by, and whether
/// it points open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fold {
    pub name: String,
    pub open: bool,
}

/// The forest a request asked for, whether it shows the bead asked for, and
/// what the page's links carry on.
pub struct Shown {
    pub forest: Forest,
    /// Nothing where no bead was asked for, and `false` where the forest
    /// draws no such bead.
    pub bead: Option<bool>,
    /// What was asked, with the folds opened to reach the bead pointed open,
    /// so the next page keeps them open as the terminal does.
    pub asked: Asked,
    /// What was asked, as a step through the search's matches carries it
    /// on: without the folds the search opened, which the next step shuts
    /// as `n` does. Every other link carries them, as every other key keeps
    /// them open.
    pub searched: Asked,
    /// Where the search asked for came to, with the selection on the match
    /// where it found one.
    pub landed: Option<Landed>,
}

/// Where the page shows one bead: its project's name as the config gives it,
/// then its id, each percent-encoded. A client that knows a bead's project
/// and id composes this and nothing else.
pub fn address(key: &BeadKey) -> String {
    format!("/{}/{}", encoded(&key.project), encoded(&key.id))
}

/// The last segment of the address a bead's comment form posts to.
pub const COMMENT: &str = "comment";

/// The writes a writable page can offer on a bead's page, each with the last
/// segment of the address its form posts to.
/// Release is left to the terminal: bdi offers it only where it flags a
/// claim whose agent is gone, and `bdi web` asks no agent provider, so it
/// never flags one.
pub const OFFERED: [(Write, &str); 7] = [
    (Write::Comment, COMMENT),
    (Write::Close, "close"),
    (Write::Reopen, "reopen"),
    (Write::Respond, "respond"),
    (Write::Dismiss, "dismiss"),
    (Write::Defer, "defer"),
    (Write::Priority, "priority"),
];

/// The write a page offers whose form posts to an address ending in `named`.
pub fn offered(named: &str) -> Option<Write> {
    OFFERED
        .iter()
        .find(|(_, name)| *name == named)
        .map(|(write, _)| *write)
}

/// The field a write's words are posted in.
pub const TEXT: &str = "text";

/// The longest words any write's form takes, in the UTF-16 units a browser
/// counts. A comment is the longest a write is given.
pub const LONGEST_COMMENT: usize = 7000;

/// The most bytes a browser posts a comment's form as: the comment and the
/// form's token. One UTF-16 unit is at most three bytes of UTF-8, and a form
/// sends each of those as `%XX`.
pub const LARGEST_COMMENT_FORM: usize = TEXT.len()
    + "=".len()
    + LONGEST_COMMENT * 9
    + "&".len()
    + TOKEN.len()
    + "=".len()
    + TOKEN_LENGTH;

/// Where a form on `key`'s page posts the write named `write`.
pub fn writing(key: &BeadKey, write: &str) -> String {
    format!("{}/{write}", address(key))
}

/// The bead and the write's name a post to `path` asks for, as [`writing`]
/// composes it, or nothing where `path` is not such an address.
pub fn written(path: &str) -> Option<(BeadKey, &str)> {
    match path.strip_prefix('/')?.split('/').collect::<Vec<_>>()[..] {
        [project, id, write] if !project.is_empty() && !id.is_empty() => Some((
            BeadKey {
                project: decoded(project)?,
                id: decoded(id)?,
            },
            write,
        )),
        _ => None,
    }
}

/// The page a write to `key` posted to `target` sends the reader back to:
/// the bead's, carrying what the form's address carried.
pub fn written_back(key: &BeadKey, target: &str) -> String {
    let query = target
        .split_once('?')
        .map_or(String::new(), |(_, query)| format!("?{query}"));
    format!("{}{query}#{SHOWN}", address(key))
}

/// The value of the field `name` in a form's body as a browser posts it,
/// or nothing where the body does not hold the field or it will not decode.
pub fn field(body: &str, name: &str) -> Option<String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .and_then(|(_, value)| form_decoded(value))
}

/// The field a write's form carries the token its page issued in.
pub const TOKEN: &str = "token";

/// How long a token is: an HMAC-SHA256 in unpadded URL-safe base64, which a
/// browser posts as it is.
pub const TOKEN_LENGTH: usize = 43;

/// What issues the token a writable page puts in each write's form, and
/// checks the token a post carries. A page on another site cannot read one,
/// so a form it posts carries none that checks. Each run has its own key,
/// and a form drawn by an earlier run carries a token this run never issued.
#[derive(Clone, Copy)]
pub struct Tokens {
    key: [u8; 32],
}

/// Without the key, which a log or a panic has no business holding.
impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tokens").finish_non_exhaustive()
    }
}

impl Tokens {
    /// Tokens under a key read from the system's random source.
    pub fn new() -> std::io::Result<Self> {
        use std::io::Read;
        let mut key = [0; 32];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut key)?;
        Ok(Self { key })
    }

    #[cfg(test)]
    pub fn keyed(key: [u8; 32]) -> Self {
        Self { key }
    }

    /// The token for the form on `bead`'s page that makes the write `write`.
    pub fn issue(&self, bead: &BeadKey, write: &str) -> String {
        BASE64_URL_SAFE_NO_PAD.encode(self.signing(bead, write).finalize().into_bytes())
    }

    /// Whether `token` is the one [`issue`](Self::issue) gives for the form
    /// on `bead`'s page that makes `write`.
    pub fn issued(&self, token: &str, bead: &BeadKey, write: &str) -> bool {
        BASE64_URL_SAFE_NO_PAD
            .decode(token)
            .is_ok_and(|digest| self.signing(bead, write).verify_slice(&digest).is_ok())
    }

    /// The HMAC over `bead` and `write`, each part led by its length so no
    /// two forms sign the same bytes.
    fn signing(&self, bead: &BeadKey, write: &str) -> Hmac<Sha256> {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("an HMAC takes a key of any length");
        for part in [&bead.project, &bead.id, write] {
            mac.update(&part.len().to_be_bytes());
            mac.update(part.as_bytes());
        }
        mac
    }
}

/// Whether a bead of `project` has an address a browser keeps as it is
/// written. A browser drops a `.` or `..` segment from a path before asking,
/// and an empty first segment makes the address name a host.
pub fn addressable(project: &str) -> bool {
    !matches!(project, "" | "." | "..")
}

impl Asked {
    /// What a request for `target` asks for of a `bdi web` started on
    /// nothing.
    #[cfg(test)]
    pub fn of(target: &str) -> Option<Self> {
        Self::started_on(target, &Start::default())
    }

    /// What a request for `target`, a path and its query, asks for: the
    /// forest at `/`, and one bead at its [`address`], each as `start` says
    /// where the address does not. Nothing where no page is at that address.
    pub fn started_on(target: &str, start: &Start) -> Option<Self> {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let (mut folds, mut place, mut root) = (Vec::new(), None, Root::AsStarted);
        let (mut view, mut ready, mut find, mut step) = (None, None, None, None);
        let (mut check, mut left, mut project) = (false, false, None);
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            if ![
                OPEN, SHUT, AT, VIEW, READY, CHECK, LEFT, PROJECT, ROOT, FIND, STEP,
            ]
            .contains(&key)
            {
                continue;
            }
            let value = form_decoded(value)?;
            let open = match key {
                OPEN => true,
                SHUT => false,
                AT => {
                    place = unplaced(&value);
                    continue;
                }
                VIEW => {
                    view = Some(value);
                    continue;
                }
                READY => {
                    ready = Some(value == ONLY);
                    continue;
                }
                CHECK => {
                    check = value == ONLY;
                    continue;
                }
                LEFT => {
                    left = value == ONLY;
                    continue;
                }
                PROJECT => {
                    project = Some(value);
                    continue;
                }
                ROOT => {
                    root = unplaced(&value).map_or(Root::Whole, Root::At);
                    continue;
                }
                FIND => {
                    find = Some(value).filter(|text| !text.is_empty());
                    continue;
                }
                STEP => {
                    step = match value.as_str() {
                        NEXT => Some(true),
                        BACK => Some(false),
                        _ => None,
                    };
                    continue;
                }
                _ => continue,
            };
            pointed(&mut folds, value, open);
        }
        let bead = match path.strip_prefix('/')?.split('/').collect::<Vec<_>>()[..] {
            [""] => None,
            [project, id] if !project.is_empty() && !id.is_empty() => Some(BeadKey {
                project: decoded(project)?,
                id: decoded(id)?,
            }),
            _ => return None,
        };
        let list_named = check || left || view.is_some() || ready.is_some();
        Some(Self {
            bead,
            folds,
            place,
            list: match (check, left, project) {
                _ if !list_named => start.list.clone(),
                (true, _, project) => Some(List::ToCheck { project }),
                (false, true, Some(project)) => Some(List::LeftToDo { project }),
                _ => listed(view, ready.unwrap_or_default()),
            },
            list_from_start: !list_named && start.list.is_some(),
            root,
            find: find.map(|text| Find { text, step }),
            start: start.clone(),
        })
    }

    /// The forest this asks for, drawn from `snapshot` under `layout`: its
    /// folds pointed, and the selection on the bead asked for, with the lines
    /// over the bead open as the terminal's go-to leaves them.
    ///
    /// A fold naming a line the forest no longer draws is passed over, as a
    /// refresh passes over a fold on a line that has gone, and is still
    /// carried on.
    ///
    /// A bead's address is how anything that knows the bead links to it, so
    /// where what this took from the start leaves the bead out, the start
    /// gives way as little as shows the bead: its list, then its roots, then
    /// both. Where none of that shows it, the page stays as the start draws
    /// it.
    pub fn forest(&self, snapshot: Snapshot, layout: &Layout) -> Shown {
        let shown = self.drawn(snapshot, layout);
        let unstarted = self.without_the_starts_list().without_the_starts_roots();
        let held = |key: &BeadKey| shown.forest.snapshot().node(key).is_some();
        if shown.bead != Some(false) || unstarted == *self || !self.bead.as_ref().is_some_and(held)
        {
            return shown;
        }
        let mut tried = self.clone();
        // ponytail: the forest is flattened again for each thing the start
        // gives way; undo the start on the first one if these pages come to
        // cost too much.
        let wider = [
            self.without_the_starts_list(),
            self.without_the_starts_roots(),
            unstarted,
        ];
        for wider in wider {
            if wider == *self || wider == tried {
                continue;
            }
            let found = wider.drawn(shown.forest.snapshot().clone(), layout);
            if found.bead == Some(true) {
                return found;
            }
            tried = wider;
        }
        shown
    }

    /// This with the list it took from the start put back to the trees.
    fn without_the_starts_list(&self) -> Self {
        Self {
            list: self.list.clone().filter(|_| !self.list_from_start),
            list_from_start: false,
            ..self.clone()
        }
    }

    /// This with the roots it took from the start put back to every tree.
    fn without_the_starts_roots(&self) -> Self {
        let root = match &self.root {
            Root::AsStarted if !self.start.roots.is_empty() => Root::Whole,
            root => root.clone(),
        };
        Self {
            root,
            ..self.clone()
        }
    }

    /// [`Asked::forest`], with nothing taken from the start given way.
    fn drawn(&self, snapshot: Snapshot, layout: &Layout) -> Shown {
        let mut forest = flatten(snapshot);
        forest.laid_out_to(layout.clone());
        match &self.root {
            Root::At(root) => {
                if forest.go_to_place(root) {
                    forest.apply(Action::FocusForest);
                }
            }
            Root::AsStarted if !self.start.roots.is_empty() => {
                forest.focus_when_drawn(self.start.roots.clone());
            }
            Root::AsStarted | Root::Whole => {}
        }
        if self.list.is_some() {
            forest.draw_list(self.list.clone());
        }
        point(&mut forest, &self.folds, &[]);
        let mut asked = self.clone();
        let open_already: Vec<String> = in_sight(&forest)
            .into_iter()
            .filter(|(_, (_, open))| *open)
            .map(|(name, _)| name)
            .collect();
        let bead = self.bead.as_ref().map(|key| self.reach(&mut forest, key));
        let mut kept_open = Vec::new();
        if let (Some(true), Some(key)) = (bead, &self.bead) {
            kept_open = folds_over_selected(&forest);
            // Going to the bead brought lines into sight whose folds the
            // reader may have pointed, and the terminal keeps those as they
            // were pointed. Pointing them moves the selection, so the bead
            // is gone to again.
            point(&mut forest, &self.folds, &kept_open);
            self.reach(&mut forest, key);
        }
        let searched = asked.clone();
        let landed = match (&self.bead, &self.find) {
            (None, Some(find)) => Some(self.search(&mut forest, find)),
            _ => None,
        };
        if let Some(Landed::On { .. }) = landed {
            kept_open = folds_over_selected(&forest);
        }
        for name in kept_open {
            if !open_already.contains(&name) {
                pointed(&mut asked.folds, name, true);
            }
        }
        Shown {
            forest,
            bead,
            asked,
            searched,
            landed,
        }
    }

    /// Search `forest` for `find`'s text from the copy the reader was on, as
    /// the terminal's search counts from the selection, and from the top
    /// where the page they searched from marked none. Or step on from the
    /// match the reader was on, as the terminal's next-match keys do.
    ///
    /// The folds the reader pointed under a line the search opened come into
    /// sight only once it has landed, so they are pointed then, holding open
    /// the lines over the match, and the match is stepped to again.
    fn search(&self, forest: &mut Forest, find: &Find) -> Landed {
        match (find.step, &self.place) {
            (None, Some(place)) => {
                forest.go_to_place(place);
            }
            (None, None) => {
                forest.select_line(0);
            }
            (Some(_), _) => {}
        }
        let origin = forest.origin();
        let mut landed = forest.seek(&find.text, &origin);
        if let (Some(forward), Some(place)) = (find.step, &self.place) {
            if forest.step_to_place(place) {
                landed = forest.next_match(forward).unwrap_or(landed);
            }
        }
        if let (Landed::On { .. }, Some(on)) = (&landed, forest.place().cloned()) {
            point(forest, &self.folds, &folds_over_selected(forest));
            forest.step_to_place(&on);
        }
        landed
    }

    /// Put the selection on `key`: on the copy the reader followed where it
    /// is still drawn, as the terminal keeps the copy it selected, and
    /// wherever the forest draws the bead otherwise.
    fn reach(&self, forest: &mut Forest, key: &BeadKey) -> bool {
        let followed = self.place.as_ref().filter(|place| place.key() == key);
        followed.is_some_and(|place| forest.go_to_place(place)) || forest.go_to(key)
    }

    /// The address that points `line`'s fold the other way, landing on the
    /// line by its [`fold_name`], where `line` folds at all. `listed` is
    /// whether the line is a row of a list drawn in place of the trees.
    ///
    /// The terminal points a fold on the selected line, and its bead view is
    /// over the selection, so on a bead's page the link goes to the page of
    /// the bead on the line, or to the forest where the line is no bead.
    pub fn toggling(&self, line: &Line, listed: bool) -> Option<String> {
        let name = fold_name(line, listed)?;
        let mut folds = self.folds.clone();
        pointed(&mut folds, name.clone(), line.folded != Some(true));
        let place = match (&self.bead, &line.content) {
            (Some(_), Content::Bead(_)) => line.place.as_ref(),
            _ => None,
        };
        let path = place.map_or_else(|| "/".to_string(), |place| address(place.key()));
        let toggled = Self {
            folds,
            ..self.clone()
        };
        Some(format!("{path}{}#{name}", toggled.query(place)))
    }

    /// The address of the page showing `key`, with the folds this page has.
    pub fn opening(&self, key: &BeadKey) -> String {
        format!("{}{}#{SHOWN}", address(key), self.query(None))
    }

    /// The address of the page showing the bead at `place`, landing on that
    /// copy of it, with the folds this page has.
    pub fn opening_at(&self, place: &Place) -> String {
        format!(
            "{}{}#{SHOWN}",
            address(place.key()),
            self.query(Some(place))
        )
    }

    /// The address a write to `key` posts to from this page, carrying what
    /// the page's own links carry.
    pub fn writing(&self, key: &BeadKey, write: &str) -> String {
        format!("{}{}", writing(key, write), self.query(self.place.as_ref()))
    }

    /// The address of the forest with no bead shown, with the folds this
    /// page has, landing on the row of the bead at `shown` where there is
    /// one. `listed` is whether the rows are a list's.
    pub fn closing(&self, shown: Option<&Place>, listed: bool) -> String {
        let landing =
            shown.map_or_else(String::new, |place| format!("#{}", row_name(place, listed)));
        format!("/{}{landing}", self.query(None))
    }

    /// The address of the page drawing `list` in place of the trees, or the
    /// trees where it is none, with the folds and the root this page has.
    pub fn listing(&self, list: Option<List>) -> String {
        let listed = Self {
            list,
            ..self.clone()
        };
        format!("/{}", listed.query(None))
    }

    /// The address of the page listing only ready beads where this one lists
    /// all of them, and all of them where it lists only ready ones, in the
    /// view this page draws, as the ready key toggles it.
    pub fn toggling_ready(&self) -> String {
        let list = self.list.as_ref();
        self.listing(listed(view_of(list).map(str::to_string), !only_ready(list)))
    }

    /// The address of this page with the forest drawn with `root` as its
    /// only root, or drawn whole where it is none.
    pub fn rooting(&self, root: Option<&Place>) -> String {
        let rooted = Self {
            root: root.cloned().map_or(Root::Whole, Root::At),
            ..self.clone()
        };
        match &self.bead {
            Some(key) => format!(
                "{}{}#{SHOWN}",
                address(key),
                rooted.query(self.place.as_ref())
            ),
            None => format!("/{}", rooted.query(None)),
        }
    }

    /// The address of the match after the one at `place` of what was
    /// searched for, or of the one before it.
    pub fn stepping(&self, find: &str, place: &Place, forward: bool) -> String {
        let step = if forward { NEXT } else { BACK };
        let mut query = self.query(Some(place));
        query.push_str(&format!("&{FIND}={}&{STEP}={step}", encoded(find)));
        format!("/{query}#{SHOWN}")
    }

    /// The pairs carrying this page's folds, list and root, and the copy at
    /// `place`. A search's form carries them beside what is typed into it,
    /// so its answer keeps them and counts from the copy, as the terminal's
    /// search counts from the selection.
    pub fn carried(&self, place: Option<&Place>) -> Vec<(&'static str, String)> {
        // The start's own list is left to the start, so a page reached from
        // it still gives way where the list would leave its bead out.
        let started = self.list_from_start && self.list == self.start.list;
        let list = self.list.as_ref().filter(|_| !started);
        let folds = self.folds.iter().map(|fold| {
            let key = if fold.open { OPEN } else { SHUT };
            (key, fold.name.clone())
        });
        let trees_over_a_start = !started && list.is_none() && self.start.list.is_some();
        let root = match &self.root {
            Root::At(root) => Some(placed(root)),
            Root::Whole if !self.start.roots.is_empty() => Some(String::new()),
            Root::AsStarted | Root::Whole => None,
        };
        folds
            .chain(view_of(list).map(|name| (VIEW, name.to_string())))
            .chain(only_ready(list).then(|| (READY, ONLY.to_string())))
            .chain(trees_over_a_start.then(|| (READY, TREES.to_string())))
            .chain(kept_to_a_project(list))
            .chain(root.map(|root| (ROOT, root)))
            .chain(place.map(|place| (AT, placed(place))))
            .collect()
    }

    /// The beads the forest is asked to be drawn with as its only roots.
    pub fn roots(&self) -> BTreeSet<BeadKey> {
        match &self.root {
            Root::AsStarted => self.start.roots.iter().cloned().collect(),
            Root::Whole => BTreeSet::new(),
            Root::At(place) => BTreeSet::from([place.key().clone()]),
        }
    }

    /// The query carrying this page's folds, list and root, and the copy
    /// at `place`, or nothing where there are none of them.
    fn query(&self, place: Option<&Place>) -> String {
        let pairs: Vec<String> = self
            .carried(place)
            .into_iter()
            .map(|(key, value)| format!("{key}={}", encoded(&value)))
            .collect();
        if pairs.is_empty() {
            return String::new();
        }
        format!("?{}", pairs.join("&"))
    }
}

/// The pairs naming `list` where it is the list of the beads to check or of
/// those left to do, with the project it keeps to.
fn kept_to_a_project(list: Option<&List>) -> Vec<(&'static str, String)> {
    let (key, project) = match list {
        Some(List::ToCheck { project }) => (CHECK, project.as_deref()),
        Some(List::LeftToDo { project }) => (LEFT, Some(project.as_str())),
        _ => return Vec::new(),
    };
    iter::once((key, ONLY.to_string()))
        .chain(project.map(|project| (PROJECT, project.to_string())))
        .collect()
}

/// `folds` with the fold `name` names pointed `open`, in place of any way it
/// pointed before.
fn pointed(folds: &mut Vec<Fold>, name: String, open: bool) {
    folds.retain(|fold| fold.name != name);
    folds.push(Fold { name, open });
}

/// What the lines open over the selection are known by, which are the folds
/// going to it opened. A list's rows are peers, so none is over another.
fn folds_over_selected(forest: &Forest) -> Vec<String> {
    if forest.list().is_some() {
        return Vec::new();
    }
    let lines = forest.lines();
    forest
        .lines_over_selected()
        .into_iter()
        .filter_map(|at| lines.get(at).and_then(|line| fold_name(line, false)))
        .collect()
}

/// Point each of `folds` on `forest` but those `held_open` names.
///
/// A fold is pointed through the forest's own fold action, which acts on a
/// line in sight, so the folds are gone through again for as long as
/// pointing one brings another into sight.
fn point(forest: &mut Forest, folds: &[Fold], held_open: &[String]) {
    let mut unseen: Vec<&Fold> = folds
        .iter()
        .filter(|fold| !held_open.contains(&fold.name))
        .collect();
    loop {
        let before = unseen.len();
        let mut known = in_sight(forest);
        unseen.retain(|fold| {
            let Some(&(at, open)) = known.get(&fold.name) else {
                return true;
            };
            if open != fold.open {
                forest.select_line(at);
                forest.apply(Action::ToggleFold);
                known = in_sight(forest);
            }
            false
        });
        if unseen.len() == before {
            break;
        }
    }
}

/// Every line in sight that folds, by what it is known by: where it is
/// drawn, and whether its fold is open.
fn in_sight(forest: &Forest) -> HashMap<String, (usize, bool)> {
    let lines = forest.lines();
    let listed = forest.list().is_some();
    lines
        .viewport(0, lines.len())
        .filter_map(|(at, line)| Some((fold_name(line, listed)?, (at, line.folded?))))
        .collect()
}

/// What a line that folds is known by in an address: what the forest knows
/// its fold by, written out. Nothing for a line with no fold.
///
/// It is the line's anchor on the page too, so a link to the line lands on
/// it wherever it is drawn. Being written as [`encoded`] writes, it stands
/// in a fragment and an id as it is.
///
/// A bead's row in a list folds to what the bead unblocks, which the forest
/// keeps by the bead and apart from the bead's folds in the trees, so where
/// the line is `listed` it is known by the bead under another name.
pub fn fold_name(line: &Line, listed: bool) -> Option<String> {
    line.folded?;
    Some(match &line.content {
        Content::Project(project) => format!("p:{}", encoded(&project.project)),
        Content::Group(group) => format!(
            "g:{:?}:{}",
            group.kind,
            group.project.as_deref().map(encoded).unwrap_or_default()
        ),
        Content::Elided { under, .. } => format!("e:{}", placed(under)),
        _ => row_name(line.place.as_ref()?, listed),
    })
}

/// The anchor `line` is drawn with on the page: a bead's own row is known by
/// its bead whether or not it folds, a row of what a bead waits on by the
/// beads stepped up through to reach it, and any other line by its fold's
/// name.
pub fn line_name(line: &Line, listed: bool) -> Option<String> {
    match &line.content {
        Content::Bead(_) => line.place.as_ref().map(|place| row_name(place, listed)),
        Content::Waiting(step) => Some(format!("w:{}", keyed(&step.up))),
        _ => fold_name(line, listed),
    }
}

/// What the row of the bead at `place` is known by, in a list where `listed`.
fn row_name(place: &Place, listed: bool) -> String {
    if listed {
        let key = place.key();
        format!("u:{}:{}", encoded(&key.project), encoded(&key.id))
    } else {
        format!("b:{}", placed(place))
    }
}

/// A place as the beads stepped through to reach it, from its tree's root.
fn placed(place: &Place) -> String {
    keyed(iter::once(&place.tree).chain(&place.steps))
}

/// `keys` written one after another, as [`placed`] writes a place's.
fn keyed<'a>(keys: impl IntoIterator<Item = &'a BeadKey>) -> String {
    keys.into_iter()
        .map(|key| format!("{}:{}", encoded(&key.project), encoded(&key.id)))
        .collect::<Vec<_>>()
        .join("/")
}

/// What [`placed`] wrote, read back.
fn unplaced(text: &str) -> Option<Place> {
    let mut keys = text.split('/').map(|step| {
        let (project, id) = step.split_once(':')?;
        Some(BeadKey {
            project: decoded(project)?,
            id: decoded(id)?,
        })
    });
    Some(Place {
        tree: keys.next()??,
        steps: keys.collect::<Option<_>>()?,
    })
}

/// `text` with every byte but the ones a URL leaves unreserved written as
/// `%XX`, so it can stand as one segment of a path or one value of a query.
///
/// `~` is unreserved too, but is written as `%7E`, because a browser takes
/// `:~:` in a fragment for a directive and finds no line by its anchor.
fn encoded(text: &str) -> String {
    let mut said = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._".contains(&byte) {
            said.push(char::from(byte));
        } else {
            said.push_str(&format!("%{byte:02X}"));
        }
    }
    said
}

/// A query's value read back, as [`encoded`] writes it or as a browser
/// writes what is typed into a form, with a space as `+`. [`encoded`] writes
/// a `+` escaped, so one standing bare is a form's space.
fn form_decoded(text: &str) -> Option<String> {
    decoded(&text.replace('+', " "))
}

/// What [`encoded`] wrote, read back. Nothing where an escape is not two hex
/// digits or the bytes are not UTF-8.
fn decoded(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut rest = text.as_bytes();
    while let Some((&byte, after)) = rest.split_first() {
        if byte == b'%' {
            let hex = after
                .get(..2)
                .filter(|hex| hex.iter().all(u8::is_ascii_hexdigit))?;
            bytes.push(u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?);
            rest = &after[2..];
        } else {
            bytes.push(byte);
            rest = after;
        }
    }
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A post to the address a bead's form posts a write to names that bead
    /// and that write, whatever its project and id are spelled with.
    #[test]
    fn a_write_posted_where_a_form_posts_it_names_its_bead_and_write() {
        let key = BeadKey {
            project: "summit works/2".to_string(),
            id: "smt-4kd3p.6~".to_string(),
        };
        assert_eq!(
            written(&writing(&key, COMMENT)),
            Some((key.clone(), COMMENT))
        );
        assert_eq!(written(&address(&key)), None);
        assert_eq!(written("/summit-works//comment"), None);
    }

    /// A form's field is read as a browser posts it, and a field it does not
    /// hold, or cannot decode, is nothing.
    #[test]
    fn a_forms_field_is_read_as_a_browser_posts_it() {
        let form = "said=no&text=lift+the+gate%2C+then+%2B1&text=again";
        assert_eq!(field(form, TEXT).as_deref(), Some("lift the gate, then +1"));
        assert_eq!(field(form, "reason"), None);
        assert_eq!(field("text=%zz", TEXT), None);
    }

    /// A token is issued for one bead and one write, and checks for that
    /// form alone, issued by those tokens alone.
    #[test]
    fn a_token_checks_only_for_the_form_it_was_issued_in() {
        let tokens = Tokens::keyed([7; 32]);
        let gate = BeadKey {
            project: "arkham".to_string(),
            id: "ark-1".to_string(),
        };
        let token = tokens.issue(&gate, COMMENT);
        assert_eq!(token.len(), TOKEN_LENGTH);
        assert!(tokens.issued(&token, &gate, COMMENT));

        let elsewhere = BeadKey {
            id: "ark-2".to_string(),
            ..gate.clone()
        };
        assert!(!tokens.issued(&token, &elsewhere, COMMENT));
        assert!(!tokens.issued(&token, &gate, "close"));
        assert!(!Tokens::keyed([8; 32]).issued(&token, &gate, COMMENT));
        assert!(!tokens.issued("", &gate, COMMENT));
        assert!(!tokens.issued("not base64!", &gate, COMMENT));
    }

    /// A project and an id are told apart however they are spelled, so the
    /// token one bead's form carries does not check for another's.
    #[test]
    fn a_token_tells_where_the_project_ends_and_the_id_starts() {
        let tokens = Tokens::keyed([7; 32]);
        let split = |project: &str, id: &str| BeadKey {
            project: project.to_string(),
            id: id.to_string(),
        };
        let token = tokens.issue(&split("ark", "ham/ark-1"), COMMENT);
        assert!(!tokens.issued(&token, &split("arkham", "/ark-1"), COMMENT));
    }

    /// Two runs issue different tokens for the same form.
    #[test]
    fn each_run_issues_its_own_tokens() {
        let gate = BeadKey {
            project: "arkham".to_string(),
            id: "ark-1".to_string(),
        };
        let token = Tokens::new().expect("a key").issue(&gate, COMMENT);
        assert!(!Tokens::new().expect("a key").issued(&token, &gate, COMMENT));
    }
}
