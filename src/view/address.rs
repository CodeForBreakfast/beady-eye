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

use std::collections::HashMap;
use std::iter;

use crate::model::join::BeadKey;
use crate::model::snapshot::Snapshot;
use crate::view::forest::{flatten, listed, only_ready, view_of, Forest, Landed, List};
use crate::view::lines::{Content, Line, Place};
use crate::view::row::Layout;
use crate::view::Action;

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

/// The query key naming the bead the forest is drawn with as its only root.
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
    /// The bead the forest is drawn with as its only root, where it is.
    pub root: Option<Place>,
    /// What was searched for, where a page asks for a search's answer
    /// rather than for a bead.
    pub find: Option<Find>,
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

/// Whether a bead of `project` has an address a browser keeps as it is
/// written. A browser drops a `.` or `..` segment from a path before asking,
/// and an empty first segment makes the address name a host.
pub fn addressable(project: &str) -> bool {
    !matches!(project, "" | "." | "..")
}

impl Asked {
    /// What a request for `target`, a path and its query, asks for: the
    /// forest at `/`, and one bead at its [`address`]. Nothing where no page
    /// is at that address.
    pub fn of(target: &str) -> Option<Self> {
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let (mut folds, mut place, mut root) = (Vec::new(), None, None);
        let (mut view, mut ready, mut find, mut step) = (None, false, None, None);
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let value = form_decoded(value)?;
            let open = match key {
                OPEN => true,
                SHUT => false,
                AT => {
                    place = unplaced(&value);
                    continue;
                }
                VIEW => {
                    view = Some(value).filter(|name| !name.is_empty());
                    continue;
                }
                READY => {
                    ready = value == ONLY;
                    continue;
                }
                ROOT => {
                    root = unplaced(&value);
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
        Some(Self {
            bead,
            folds,
            place,
            list: listed(view, ready),
            root,
            find: find.map(|text| Find { text, step }),
        })
    }

    /// The forest this asks for, drawn from `snapshot` under `layout`: its
    /// folds pointed, and the selection on the bead asked for, with the lines
    /// over the bead open as the terminal's go-to leaves them.
    ///
    /// A fold naming a line the forest no longer draws is passed over, as a
    /// refresh passes over a fold on a line that has gone, and is still
    /// carried on.
    pub fn forest(&self, snapshot: Snapshot, layout: &Layout) -> Shown {
        let mut forest = flatten(snapshot);
        forest.laid_out_to(layout.clone());
        if let Some(root) = &self.root {
            if forest.go_to_place(root) {
                forest.apply(Action::FocusForest);
            }
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
        if let (Some(true), Some(key)) = (bead, &self.bead) {
            let lines = forest.lines();
            let over: Vec<String> = forest
                .lines_over_selected()
                .into_iter()
                .filter_map(|at| lines.get(at).and_then(fold_name))
                .collect();
            // Going to the bead brought lines into sight whose folds the
            // reader may have pointed, and the terminal keeps those as they
            // were pointed. Pointing them moves the selection, so the bead
            // is gone to again.
            point(&mut forest, &self.folds, &over);
            self.reach(&mut forest, key);
            for name in over {
                if !open_already.contains(&name) {
                    pointed(&mut asked.folds, name, true);
                }
            }
        }
        let landed = match (&self.bead, &self.find) {
            (None, Some(find)) => Some(self.search(&mut forest, find)),
            _ => None,
        };
        Shown {
            forest,
            bead,
            asked,
            landed,
        }
    }

    /// Search `forest` for `find`'s text as the terminal's search does, or
    /// step on from the match the reader was on as its next-match keys do.
    fn search(&self, forest: &mut Forest, find: &Find) -> Landed {
        let origin = forest.origin();
        let landed = forest.seek(&find.text, &origin);
        let (Some(forward), Some(place)) = (find.step, &self.place) else {
            return landed;
        };
        if !forest.go_to_place(place) {
            return landed;
        }
        forest.next_match(forward).unwrap_or(landed)
    }

    /// Put the selection on `key`: on the copy the reader followed where it
    /// is still drawn, as the terminal keeps the copy it selected, and
    /// wherever the forest draws the bead otherwise.
    fn reach(&self, forest: &mut Forest, key: &BeadKey) -> bool {
        let followed = self.place.as_ref().filter(|place| place.key() == key);
        followed.is_some_and(|place| forest.go_to_place(place)) || forest.go_to(key)
    }

    /// The address that points `line`'s fold the other way, landing on the
    /// line, where `line` folds at all. `at` is where the line is drawn,
    /// which pointing its own fold does not move.
    ///
    /// The terminal points a fold on the selected line, and its bead view is
    /// over the selection, so on a bead's page the link goes to the page of
    /// the bead on the line, or to the forest where the line is no bead.
    pub fn toggling(&self, line: &Line, at: usize) -> Option<String> {
        let name = fold_name(line)?;
        let mut folds = self.folds.clone();
        pointed(&mut folds, name, line.folded != Some(true));
        let place = match (&self.bead, &line.content) {
            (Some(_), Content::Bead(_)) => line.place.as_ref(),
            _ => None,
        };
        let path = place.map_or_else(|| "/".to_string(), |place| address(place.key()));
        let toggled = Self {
            folds,
            ..self.clone()
        };
        Some(format!(
            "{path}{}#{}",
            toggled.query(place),
            line_anchor(at)
        ))
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

    /// The address of the forest with no bead shown, with the folds this
    /// page has.
    pub fn closing(&self) -> String {
        format!("/{}", self.query(None))
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
            root: root.cloned(),
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

    /// The pairs a search's form carries beside what is typed into it, so
    /// the page it answers with keeps this page's folds, list and root.
    pub fn carried(&self) -> Vec<(&'static str, String)> {
        let list = self.list.as_ref();
        let folds = self.folds.iter().map(|fold| {
            let key = if fold.open { OPEN } else { SHUT };
            (key, fold.name.clone())
        });
        folds
            .chain(view_of(list).map(|name| (VIEW, name.to_string())))
            .chain(only_ready(list).then(|| (READY, ONLY.to_string())))
            .chain(self.root.as_ref().map(|root| (ROOT, placed(root))))
            .collect()
    }

    /// The query carrying this page's folds, list and root, and the copy
    /// at `place`, or nothing where there are none of them.
    fn query(&self, place: Option<&Place>) -> String {
        let pairs: Vec<String> = self
            .carried()
            .into_iter()
            .chain(place.map(|place| (AT, placed(place))))
            .map(|(key, value)| format!("{key}={}", encoded(&value)))
            .collect();
        if pairs.is_empty() {
            return String::new();
        }
        format!("?{}", pairs.join("&"))
    }
}

/// `folds` with the fold `name` names pointed `open`, in place of any way it
/// pointed before.
fn pointed(folds: &mut Vec<Fold>, name: String, open: bool) {
    folds.retain(|fold| fold.name != name);
    folds.push(Fold { name, open });
}

/// The anchor on the line drawn `at`.
pub fn line_anchor(at: usize) -> String {
    format!("l{at}")
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
    lines
        .viewport(0, lines.len())
        .filter_map(|(at, line)| Some((fold_name(line)?, (at, line.folded?))))
        .collect()
}

/// What a line that folds is known by in an address: what the forest knows
/// its fold by, written out. Nothing for a line with no fold.
fn fold_name(line: &Line) -> Option<String> {
    line.folded?;
    Some(match &line.content {
        Content::Project(project) => format!("p:{}", encoded(&project.project)),
        Content::Group(group) => format!(
            "g:{:?}:{}",
            group.kind,
            group.project.as_deref().map(encoded).unwrap_or_default()
        ),
        Content::Elided { under, .. } => format!("e:{}", placed(under)),
        _ => format!("b:{}", placed(line.place.as_ref()?)),
    })
}

/// A place as the beads stepped through to reach it, from its tree's root.
fn placed(place: &Place) -> String {
    iter::once(&place.tree)
        .chain(&place.steps)
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
fn encoded(text: &str) -> String {
    let mut said = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
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
