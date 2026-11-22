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

use std::collections::HashMap;
use std::iter;

use crate::model::join::BeadKey;
use crate::model::snapshot::Snapshot;
use crate::view::forest::{flatten, Forest};
use crate::view::lines::{Content, Line, Place};
use crate::view::row::Layout;
use crate::view::Action;

/// The query keys a fold pointed open and a fold pointed shut are written
/// under.
const OPEN: &str = "open";
const SHUT: &str = "shut";

/// The query key naming which copy of a bead drawn more than once the page
/// lands on.
const AT: &str = "at";

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
        let (mut folds, mut place) = (Vec::new(), None);
        for pair in query.split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let open = match key {
                OPEN => true,
                SHUT => false,
                AT => {
                    place = unplaced(&decoded(value)?);
                    continue;
                }
                _ => continue,
            };
            pointed(&mut folds, decoded(value)?, open);
        }
        let bead = match path.strip_prefix('/')?.split('/').collect::<Vec<_>>()[..] {
            [""] => None,
            [project, id] if !project.is_empty() && !id.is_empty() => Some(BeadKey {
                project: decoded(project)?,
                id: decoded(id)?,
            }),
            _ => return None,
        };
        Some(Self { bead, folds, place })
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
        Shown {
            forest,
            bead,
            asked,
        }
    }

    /// Put the selection on `key`: on the copy the reader followed where it
    /// is still drawn, as the terminal keeps the copy it selected, and
    /// wherever the forest draws the bead otherwise.
    fn reach(&self, forest: &mut Forest, key: &BeadKey) -> bool {
        let followed = self.place.as_ref().filter(|place| place.key() == key);
        followed.is_some_and(|place| forest.go_to_place(place)) || forest.go_to(key)
    }

    /// The address that points `line`'s fold the other way, landing on the
    /// line by its [`fold_name`], where `line` folds at all.
    ///
    /// The terminal points a fold on the selected line, and its bead view is
    /// over the selection, so on a bead's page the link goes to the page of
    /// the bead on the line, or to the forest where the line is no bead.
    pub fn toggling(&self, line: &Line) -> Option<String> {
        let name = fold_name(line)?;
        let mut folds = self.folds.clone();
        pointed(&mut folds, name.clone(), line.folded != Some(true));
        let place = match (&self.bead, &line.content) {
            (Some(_), Content::Bead(_)) => line.place.as_ref(),
            _ => None,
        };
        let path = place.map_or_else(|| "/".to_string(), |place| address(place.key()));
        Some(format!("{path}{}#{name}", query(&folds, place)))
    }

    /// The address of the page showing `key`, with the folds this page has.
    pub fn opening(&self, key: &BeadKey) -> String {
        format!("{}{}#{SHOWN}", address(key), query(&self.folds, None))
    }

    /// The address of the page showing the bead at `place`, landing on that
    /// copy of it, with the folds this page has.
    pub fn opening_at(&self, place: &Place) -> String {
        format!(
            "{}{}#{SHOWN}",
            address(place.key()),
            query(&self.folds, Some(place))
        )
    }

    /// The address of the forest with no bead shown, with the folds this
    /// page has.
    pub fn closing(&self) -> String {
        format!("/{}", query(&self.folds, None))
    }
}

/// `folds` with the fold `name` names pointed `open`, in place of any way it
/// pointed before.
fn pointed(folds: &mut Vec<Fold>, name: String, open: bool) {
    folds.retain(|fold| fold.name != name);
    folds.push(Fold { name, open });
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
///
/// It is the line's anchor on the page too, so a link to the line lands on
/// it wherever it is drawn. Being written as [`encoded`] writes, it stands
/// in a fragment and an id as it is.
pub fn fold_name(line: &Line) -> Option<String> {
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

/// The query that carries `folds` and the copy at `place`, or nothing where
/// there are neither.
fn query(folds: &[Fold], place: Option<&Place>) -> String {
    let pairs: Vec<String> = folds
        .iter()
        .map(|fold| {
            let key = if fold.open { OPEN } else { SHUT };
            format!("{key}={}", encoded(&fold.name))
        })
        .chain(place.map(|place| format!("{AT}={}", encoded(&placed(place)))))
        .collect();
    if pairs.is_empty() {
        return String::new();
    }
    format!("?{}", pairs.join("&"))
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
