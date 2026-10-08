//! bd's command line as the way to a project's tracker.
//!
//! The one module that spells `bd -C <path> --readonly …`, and the two ways
//! `bdi` writes: the `bd -C <path> human respond …` that `bdi bd` passes
//! through, and the gate resolve and comment that settle a gh:pr gate. Each question the
//! seam asks is one bd invocation or two, answered in bd's own JSON and parsed
//! here and nowhere else. The roots to draw the rows under are read off the
//! rows themselves, in `app::tracker`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::Deserialize;

use chrono::{DateTime, Utc};
use serde::Deserializer;
use serde_json::Value;

use crate::collect::environment;
use crate::collect::gates::{PrGate, Wait};
use crate::collect::run::{together, Env, FailureKind, RunFailure, Runner};
use crate::collect::tracker::{OpenFailure, Tracker, Trackers};
use crate::config::Project;
use crate::model::gate;
use crate::model::types::{Bead, Dependency, Edge, Printed, Status};

/// Parse a flat array of bd rows, however the answer that carried them was
/// asked for. `bd list`, `bd ready` and `bd query` all write the same row.
#[cfg(any(test, feature = "testing"))]
pub fn parse_beads(s: &str) -> anyhow::Result<Vec<Bead>> {
    parsed(s, false)
}

/// The same rows, each bead shared as a project's read holds it.
#[cfg(any(test, feature = "testing"))]
pub fn parse_shared_beads(s: &str) -> anyhow::Result<Vec<Arc<Bead>>> {
    Ok(parse_beads(s)?.into_iter().map(Arc::new).collect())
}

/// The rows in `s` as beads, each holding the row it was read from where
/// `keeping_rows` asks for it.
fn parsed(s: &str, keeping_rows: bool) -> anyhow::Result<Vec<Bead>> {
    const SHAPE: &str = "bd --json returned a shape we do not understand";
    let written: Vec<Printed> = serde_json::from_str(s).context(SHAPE)?;
    match written
        .into_iter()
        .map(|written| bead_of(written, keeping_rows))
        .collect()
    {
        Ok(beads) => Ok(beads),
        // A row read out of its map has lost where in the answer it was, so
        // the answer is read again as rows to say where it broke.
        Err(unplaced) => Err(serde_json::from_str::<Vec<Row>>(s)
            .err()
            .unwrap_or(unplaced))
        .context(SHAPE),
    }
}

/// One row as a bead, its typed fields moved out of the map it was read into,
/// and holding the map as well where `keeping_rows` asks for it.
pub(crate) fn bead_of(written: Printed, keeping_rows: bool) -> serde_json::Result<Bead> {
    let values = values_of(&written);
    let printed = keeping_rows.then(|| Arc::new(written.clone()));
    Ok(Row::deserialize(serde_json::Value::Object(written))?.into_bead(values, printed))
}

/// Every value this row holds that no text field of `Bead` holds, under the
/// key that names it: a field by its own name, and a member of a field's
/// object by the two joined with a dot. Beside them, every list of values it
/// holds under a key named the same way.
///
/// A badge reads these through `Bead::value` and `Bead::members`, so what a
/// row holds is what a badge can draw or test. A field bd grows is drawable
/// the day bd writes it, and an object-valued one is drawable a member at a
/// time, without `bdi` learning a thing about either.
///
/// `text_of` decides what is one value, and it decides it the same way for a
/// field, for a member and for a member of a list. The rule is about kinds of
/// value rather than names of fields, so nothing here moves when bd's schema
/// does.
fn values_of(row: &serde_json::Map<String, serde_json::Value>) -> Values {
    let mut values = Values::default();
    for (field, value) in row {
        match object_written_either_way(value) {
            Some(members) => {
                for (key, member) in members.iter() {
                    values.read(format!("{field}.{key}"), member);
                }
            }
            None if Bead::TEXT_FIELDS.contains(&field.as_str()) => {}
            None => values.read(field.clone(), value),
        }
    }
    values
}

/// What `values_of` read out of one row: the values, and the lists of them.
#[derive(Default)]
struct Values {
    single: BTreeMap<String, String>,
    lists: BTreeMap<String, Vec<String>>,
}

impl Values {
    /// `value` under `key`, wherever it is one value or a list of some.
    fn read(&mut self, key: String, value: &serde_json::Value) {
        if let serde_json::Value::Array(members) = value {
            let list: Vec<String> = members.iter().filter_map(text_of).collect();
            if !list.is_empty() {
                self.lists.insert(key, list);
            }
        } else if let Some(text) = text_of(value) {
            self.single.insert(key, text);
        }
    }
}

/// One value as the text it prints as, or nothing where it is not one value.
///
/// A string, a number and a boolean are each one value. A null and an empty
/// string are both how bd spells something nothing was written to — it writes
/// the top of a chain as an empty parent — and an array is not one value at
/// all. Those are no key, which is where a name no row carries already lands.
/// So is an object, which is what keeps a key naming a whole one from drawing
/// the blob onto a row.
fn text_of(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) if !text.is_empty() => Some(text.clone()),
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) => Some(value.to_string()),
        _ => None,
    }
}

/// One row of a bd listing, in the shape bd writes it, holding only the
/// fields `bdi` reads.
///
/// Unknown fields are ignored, and a field written as null reads as the one
/// bd left out; a field of any other wrong type is still an error.
///
/// `depth` is deliberately absent: bd flattens it under `--max-depth`, so the
/// tree recomputes nesting from the dependency edges instead.
#[derive(Deserialize)]
struct Row {
    id: String,
    #[serde(deserialize_with = "null_is_default")]
    title: String,
    #[serde(deserialize_with = "null_is_unrecognised")]
    status: Status,
    #[serde(default, deserialize_with = "null_is_default")]
    priority: u8,
    #[serde(default, deserialize_with = "null_is_default")]
    issue_type: String,
    /// bd writes the top of a chain as an empty parent, or leaves the field
    /// out; either reads as none.
    #[serde(default, deserialize_with = "empty_is_none")]
    parent: Option<String>,
    #[serde(default, deserialize_with = "none_is_empty")]
    dependencies: Vec<RowDependency>,
    #[serde(default, deserialize_with = "text_of_each_value")]
    metadata: BTreeMap<String, String>,
    /// The row's `created_by`. The row's `owner` is an address and is not
    /// read: nothing draws it.
    #[serde(default)]
    created_by: Option<String>,
    #[serde(default)]
    assignee: Option<String>,
    #[serde(default, deserialize_with = "null_is_default")]
    labels: Vec<String>,
    /// As `bd show` prints it. bd leaves the field out of a row that has
    /// none.
    #[serde(default)]
    description: Option<Arc<str>>,
    /// Everything `bd note` has added, as one text. Left out the same way.
    #[serde(default)]
    notes: Option<Arc<str>>,
    #[serde(default)]
    created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    started_at: Option<DateTime<Utc>>,
    #[serde(default)]
    closed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    defer_until: Option<DateTime<Utc>>,
}

/// One dependency as a row carries it.
#[derive(Deserialize)]
struct RowDependency {
    depends_on_id: String,
    #[serde(rename = "type")]
    edge: Edge,
}

impl Row {
    /// This row as a bead, beside every value a badge could name in it.
    fn into_bead(self, values: Values, printed: Option<Arc<Printed>>) -> Bead {
        let row = self;
        Bead {
            values: values.single,
            lists: values.lists,
            row: printed,
            id: row.id,
            title: row.title,
            status: row.status,
            priority: row.priority,
            issue_type: row.issue_type,
            parent: row.parent,
            dependencies: row
                .dependencies
                .into_iter()
                .map(|dependency| Dependency {
                    on: dependency.depends_on_id,
                    edge: dependency.edge,
                })
                .collect(),
            metadata: row.metadata,
            created_by: row.created_by,
            assignee: row.assignee,
            labels: row.labels,
            description: row.description,
            notes: row.notes,
            created_at: row.created_at,
            updated_at: row.updated_at,
            started_at: row.started_at,
            closed_at: row.closed_at,
            defer_until: row.defer_until,
        }
    }
}

/// bd omits a field it has nothing for, and `#[serde(default)]` covers that.
/// It does not extend to an explicit null. A tracker is read whole, so a row
/// bd wrote the other way costs not one bead's edges but every bead in that
/// project.
fn none_is_empty<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<RowDependency>, D::Error> {
    Ok(Option::<Vec<RowDependency>>::deserialize(d)?.unwrap_or_default())
}

/// bd spells an absent parent three ways — `""`, `null`, or no field — and
/// they all mean the top of a chain.
fn empty_is_none<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(Option::<String>::deserialize(d)?.filter(|parent| !parent.is_empty()))
}

/// A field bd wrote as null holds what a field bd left out holds: nothing.
fn null_is_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

/// A row whose status is null claims no status, which is outside bd's set
/// rather than any member of it. bdi says that on the screen instead of
/// picking a status the row does not claim.
fn null_is_unrecognised<'de, D: Deserializer<'de>>(d: D) -> Result<Status, D::Error> {
    Ok(Option::<Status>::deserialize(d)?.unwrap_or_else(|| Status::Other(String::new())))
}

/// A bead's metadata is whatever JSON was written into it, and bdi draws it
/// as text. So each value is read as the text it prints as, and a value that
/// is not a string costs nothing.
///
/// bd wrote the object itself as a string spelling one until April 2026, so
/// a tracker that straddles that date holds rows of both shapes and both are
/// read here. Anything else bd could write there — a null, a number, a string
/// spelling something that is not an object — is read as no metadata.
///
/// A tracker is read whole, so the alternative is not a bead without its
/// badge — it is every bead in that project, gone.
fn text_of_each_value<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<BTreeMap<String, String>, D::Error> {
    Ok(Option::<serde_json::Value>::deserialize(d)?
        .as_ref()
        .and_then(object_written_either_way)
        .map(|fields| text_of_each(fields.into_owned()))
        .unwrap_or_default())
}

/// The object a value holds, for a value that is one — either written as an
/// object, or written as a string spelling one, which is how bd wrote a bead's
/// metadata until April 2026.
fn object_written_either_way(
    value: &serde_json::Value,
) -> Option<std::borrow::Cow<'_, serde_json::Map<String, serde_json::Value>>> {
    match value {
        serde_json::Value::Object(fields) => Some(std::borrow::Cow::Borrowed(fields)),
        serde_json::Value::String(spelled) => match serde_json::from_str(spelled) {
            Ok(serde_json::Value::Object(fields)) => Some(std::borrow::Cow::Owned(fields)),
            _ => None,
        },
        _ => None,
    }
}

fn text_of_each(fields: serde_json::Map<String, serde_json::Value>) -> BTreeMap<String, String> {
    fields
        .into_iter()
        .map(|(key, value)| match value {
            serde_json::Value::String(text) => (key, text),
            written => (key, written.to_string()),
        })
        .collect()
}

/// One row of `bd blocked --json`, which carries a blocker set no dep-tree
/// row has.
#[derive(Deserialize)]
struct BlockedRow {
    id: String,
    #[serde(default, deserialize_with = "each_id_written")]
    blocked_by: Vec<String>,
}

/// The ids a blocker set holds, for a value that is an array of them.
///
/// Anything else bd could write there — a null, a lone string, an object, an
/// element that is not a string — is read as no blocker rather than as an
/// error. A tracker is read whole, so the alternative is not one bead without
/// its blockers but every blocked bead in that project, gone.
fn each_id_written<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match serde_json::Value::deserialize(d)? {
        serde_json::Value::Array(values) => values
            .into_iter()
            .filter_map(|value| match value {
                serde_json::Value::String(id) => Some(id),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    })
}

/// bd's CLI, reaching every project's tracker through one runner.
pub struct Cli<'r> {
    runner: &'r dyn Runner,
    /// The credential the shell `bdi` was launched from holds, which a
    /// project configuring none reaches its tracker on. Read once: the shell
    /// `bdi` was launched from does not change while it runs.
    ambient: Option<String>,
    /// The projects whose tracker refused the probe: bd's embedded Dolt has
    /// no server for `bd sql` to reach, and says so the same way on every
    /// refresh. Found out once per project, from the refusal itself, so the
    /// probe is paid for once per run rather than once per refresh.
    without_a_probe: Mutex<BTreeSet<String>>,
    /// Whether each bead is handed over with the row bd printed for it. Only
    /// a watcher asks: a view draws what it parsed, and a row held beside
    /// every bead would hold the tracker's text twice.
    keeping_rows: bool,
    /// Whether a project claiming an events journal has it read. Only a
    /// watcher asks, because only a watcher passes the records on.
    reading_journals: bool,
    /// Whether a finished bead is read without its free text, for a run that
    /// shows unfinished work alone.
    unfinished_work: bool,
    /// The environments captured on earlier runs, which a project is read
    /// with while direnv says entering its directory would produce the same.
    cache: Option<environment::EnvironmentCache>,
}

impl<'r> Cli<'r> {
    pub fn new(runner: &'r dyn Runner) -> Self {
        Self {
            runner,
            ambient: environment::ambient_credential(),
            without_a_probe: Mutex::default(),
            keeping_rows: false,
            reading_journals: false,
            unfinished_work: false,
            cache: None,
        }
    }

    /// The same CLI, reading the events journal of each project whose config
    /// claims one.
    pub fn reading_journals(self) -> Self {
        Self {
            reading_journals: true,
            ..self
        }
    }

    /// The same CLI, reading each project with the environment an earlier
    /// run captured for it wherever that is still what entering its
    /// directory would produce.
    pub fn caching_environments(self, cache: Option<environment::EnvironmentCache>) -> Self {
        Self { cache, ..self }
    }

    /// The same CLI, handing each bead over with the row bd printed for it.
    pub fn keeping_rows(self) -> Self {
        Self {
            keeping_rows: true,
            ..self
        }
    }

    /// The same CLI, reading every bead but a finished bead's free text, for
    /// a run that never shows that text. Every bead is still read, so every
    /// tree is placed as before.
    pub fn for_unfinished_work(self) -> Self {
        Self {
            unfinished_work: true,
            ..self
        }
    }
}

impl Cli<'_> {
    /// `project`'s tracker, opened in the environment its config asks for.
    fn reader(&self, project: &Project) -> Result<Reader<'_>, OpenFailure> {
        let env = environment::tracker_env(
            self.runner,
            project,
            self.ambient.as_deref(),
            self.cache.as_ref(),
        )?;
        Ok(Reader {
            runner: self.runner,
            name: project.name.clone(),
            path: project.path.clone(),
            env,
            without_a_probe: &self.without_a_probe,
            keeping_rows: self.keeping_rows,
            journal: self.reading_journals && project.events_journal,
            unfinished_work: self.unfinished_work,
        })
    }

    /// Every open gh:pr gate `project`'s tracker holds that is `wanted`.
    pub fn pr_gates(
        &self,
        project: &Project,
        wanted: impl Fn(&PrGate) -> bool,
    ) -> Result<Vec<PrGate>, OpenFailure> {
        Ok(self.reader(project)?.pr_gates(wanted)?)
    }

    /// `project`'s tracker, opened once to settle the gh:pr gates waiting on
    /// a pull request.
    pub fn settling(&self, project: &Project) -> Result<Settling<'_>, OpenFailure> {
        Ok(Settling(self.reader(project)?))
    }
}

/// One project's tracker opened to settle its gh:pr gates.
///
/// Settling a gate is the second of the two ways `bdi` writes to a tracker,
/// beside the `human respond` that `bdi bd` passes through. `resolve` and
/// `comment` are those writes, and everything else here is a read.
pub struct Settling<'r>(Reader<'r>);

impl Settling<'_> {
    /// Every open gh:pr gate the tracker holds whose wait is `wanted`.
    pub fn pr_gates_waiting(
        &self,
        wanted: impl Fn(&Wait) -> bool,
    ) -> Result<Vec<PrGate>, RunFailure> {
        self.0
            .pr_gates(|gate| gate.awaits.as_ref().is_ok_and(&wanted))
    }

    /// The text of every comment on `bead`, oldest first.
    pub fn comments(&self, bead: &str) -> Result<Vec<String>, RunFailure> {
        let out = self.0.asked(&["comments", bead, "--json"])?;
        let comments: Vec<Comment> = serde_json::from_str(&out)
            .map_err(|e| RunFailure::parse("bd", e).reading("comments"))?;
        Ok(comments.into_iter().map(|comment| comment.text).collect())
    }

    /// Close `gate`. `gate resolve` refuses a bead that is not a gate, so a
    /// wrong id cannot close the work itself.
    pub fn resolve(&self, gate: &str, reason: &str) -> Result<(), RunFailure> {
        self.0
            .written(&["gate", "resolve", gate, "--reason", reason])
            .map(drop)
    }

    /// Add `text` to `bead` as a comment.
    pub fn comment(&self, bead: &str, text: &str) -> Result<(), RunFailure> {
        self.0.written(&["comments", "add", bead, text]).map(drop)
    }
}

/// One comment as `bd comments --json` writes it, holding only its text.
#[derive(Deserialize)]
struct Comment {
    text: String,
}

impl Trackers for Cli<'_> {
    fn of(&self, project: &Project) -> Result<Box<dyn Tracker + '_>, OpenFailure> {
        Ok(Box::new(self.reader(project)?))
    }
}

/// One project's tracker as bd reads it: in the project's directory, with the
/// environment its config asked for.
struct Reader<'r> {
    runner: &'r dyn Runner,
    name: String,
    path: PathBuf,
    env: Env,
    /// The run's memory of which projects' trackers refused the probe,
    /// shared with every reader the run opens.
    without_a_probe: &'r Mutex<BTreeSet<String>>,
    keeping_rows: bool,
    /// Whether this tracker's events journal is read.
    journal: bool,
    unfinished_work: bool,
}

impl Reader<'_> {
    /// One answer out of the tracker.
    ///
    /// `-C` names the tracker outright, and it outranks `BEADS_DIR` in both
    /// directions: a wrong variable still resolves the project, and a wrong
    /// directory is refused rather than resolved to something plausible. That
    /// is what makes entering the directory safe, because direnv can quietly
    /// do nothing. Clearing the inherited variables stays as well; together
    /// they mean a misconfiguration fails loudly.
    ///
    /// Every subcommand composed here is a read, and that rests on the command
    /// lines below rather than on bd. It does not mean a read leaves the
    /// tracker alone, because bd writes to one on its own account. The pinned bd
    /// 1.3.0 leaves two lock files behind even on a read it answers. A bd older
    /// than 1.3.0 also rewrites `.beads/.local_version` and runs its schema
    /// auto-migration on finding itself newer than the bd that last opened that
    /// tracker, before the subcommand runs and whatever the subcommand is,
    /// `--readonly` included. `docs/design.md`'s *Reading a tracker is not
    /// leaving it alone* carries the measurement.
    /// `--readonly` vetoes bd's mutating subcommands,
    /// so a mutating call arriving here later is refused rather than run — a
    /// guard on the next edit, and a veto over subcommands rather than a
    /// property of the tracker's files. `sql` is outside even that: it is a
    /// general executor, bd's own help for it warns that direct database
    /// access bypasses the storage layer, and `--readonly` does not veto it —
    /// measured against this project's own tracker on 2026-09-01. What holds
    /// there instead is `TABLE_HASHES`, a constant nothing composes, reached
    /// from one method that takes no argument.
    fn asked(&self, subcommand: &[&str]) -> Result<String, RunFailure> {
        self.run(&["--readonly"], subcommand)
            .map_err(|failure| failure.reading(subcommand[0]))
    }

    /// One write to the tracker, which only `Settling` makes. It is `asked`
    /// without `--readonly`, so bd's veto on mutating subcommands is lifted
    /// for this call alone.
    fn written(&self, subcommand: &[&str]) -> Result<String, RunFailure> {
        self.run(&[], subcommand)
    }

    /// `subcommand` run against this tracker and no other, under `flags`.
    fn run(&self, flags: &[&str], subcommand: &[&str]) -> Result<String, RunFailure> {
        let named = self.path.to_string_lossy();
        let mut argv = vec!["-C", named.as_ref()];
        argv.extend_from_slice(flags);
        argv.extend_from_slice(subcommand);
        self.runner.run("bd", &argv, Some(&self.path), &self.env)
    }

    /// Every table's hash in the Dolt working set bar `leases`, folded into
    /// one string: it moves when anything `bdi` reads is written, committed
    /// or not.
    ///
    /// Not the committed head, because `bdi` reads wisps and the head cannot
    /// see them. `wisps` and `wisp_%` are in `dolt_ignore`, so they live in
    /// the working set and never reach `dolt_log` — measured against this
    /// project's own tracker on 2026-09-01, one `bd create --ephemeral` left
    /// `hashof('HEAD')` identical either side of it and moved the working set.
    ///
    /// Not the whole working root either, because from beads 1.3.0 a lease
    /// heartbeat writes `leases` alone, an ignored table, and so moves the
    /// root every few minutes for as long as a bead is claimed. A badge on a
    /// lease field is left to lag, as `docs/configuration.md` says. Measured
    /// 2026-09-23 on throwaway 1.3.0 and
    /// 1.2.2 stores, this held still across two heartbeats and a full read,
    /// and moved on a claim, a wisp and a plain update.
    ///
    /// A row per table rather than one `GROUP_CONCAT`, because Dolt cuts that
    /// at `group_concat_max_len`, 1024 bytes, and ignores a `SET_VAR` hint
    /// raising it; a 1.3.0 tracker already comes to 956.
    fn tables_hashed(&self) -> Result<String, RunFailure> {
        let out = self.asked(&["sql", "--json", TABLE_HASHES])?;
        let rows: Vec<TableHash> =
            serde_json::from_str(&out).map_err(|e| RunFailure::parse("bd", e).reading("sql"))?;
        if rows.is_empty() {
            return Err(RunFailure::parse("bd", "the answer holds no row").reading("sql"));
        }
        Ok(rows
            .iter()
            .map(|row| format!("{}={}", row.name, row.h))
            .collect::<Vec<_>>()
            .join(","))
    }

    /// A tracker's wisps, closed ones included.
    ///
    /// A second call, because bd keeps its ephemeral beads in a table `bd
    /// list` does not read: measured against this project's own tracker on
    /// 2026-08-31, `bd list --all` answered 120 rows both before and after two
    /// wisps were written, and `bd list --wisp-type heartbeat` answered `[]`
    /// against a heartbeat wisp that existed. `bd query` is the one call that
    /// reads them, and it writes the same row `bd list` does.
    fn wisps(&self) -> Result<String, RunFailure> {
        self.asked(&["query", EPHEMERAL, "--all", "--limit", "0", "--json"])
    }

    /// Every bead the tracker holds, wisps and gates among them.
    ///
    /// Every listing asks for gates by name, because `bd list` leaves them
    /// out otherwise: on a throwaway bd 1.3.0 tracker holding three beads
    /// and a `gh:pr` gate on each, `bd list --all` answered the three beads
    /// and `--include-gates` all six. A gate is a bead the work waits on, so
    /// a tree drawn without it says the work waits on nothing.
    fn every_bead(&self) -> Result<Vec<Bead>, RunFailure> {
        let (listed, wisps) = together(
            || self.asked(&["list", "--all", "--include-gates", "--limit", "0", "--json"]),
            || self.wisps(),
        );
        let mut beads = rows(&listed?, "list", self.keeping_rows)?;
        beads.extend(rows(&wisps?, "query", self.keeping_rows)?);
        Ok(beads)
    }

    /// Every bead the tracker holds, wisps among them, with a finished bead's
    /// free text left out: its description, its notes, and the rest of what
    /// `bd list --brief` drops.
    ///
    /// The unfinished beads come whole from bd's default listing, and the
    /// rest from a brief listing asked for at the same time.
    ///
    /// bd's default listing leaves out a status bd counts as done, and
    /// `Status::is_finished` does not count it. A tracker holding a bead in
    /// such a status is read whole instead, so that the bead keeps its text.
    /// So is a tracker whose bd predates `--brief`, which bd 1.2.0 added.
    fn every_bead_with_unfinished_text(&self) -> Result<Vec<Bead>, RunFailure> {
        let ((unfinished, briefly), wisps) = together(
            || {
                together(
                    || self.asked(&["list", "--include-gates", "--limit", "0", "--json"]),
                    || {
                        self.asked(&[
                            "list",
                            "--all",
                            "--include-gates",
                            "--brief",
                            "--limit",
                            "0",
                            "--json",
                        ])
                    },
                )
            },
            || self.wisps(),
        );
        let briefly = match briefly {
            Err(refused) if refused.kind == FailureKind::UnknownFlag => return self.every_bead(),
            briefly => briefly?,
        };
        let mut beads = rows(&unfinished?, "list", self.keeping_rows)?;
        let whole: BTreeSet<String> = beads.iter().map(|bead| bead.id.clone()).collect();
        let briefly: Vec<Bead> = rows(&briefly, "list", self.keeping_rows)?
            .into_iter()
            .filter(|bead| !whole.contains(&bead.id))
            .collect();
        if briefly.iter().any(|bead| !bead.status.is_finished()) {
            return self.every_bead();
        }
        beads.extend(briefly);
        beads.extend(rows(&wisps?, "query", self.keeping_rows)?);
        Ok(beads)
    }

    /// Every open gh:pr gate `wanted` holds of, each with the beads it holds
    /// back.
    ///
    /// `bd list` leaves gates out, and neither `bd gate list` nor `bd show`
    /// carries the beads a gate holds back. `bd dep list` takes several ids
    /// but answers one flat array that does not say which bead hangs on which
    /// id, so it is asked once per gate, and only of a gate that is wanted.
    fn pr_gates(&self, wanted: impl Fn(&PrGate) -> bool) -> Result<Vec<PrGate>, RunFailure> {
        let listed = self.asked(&["gate", "list", "--limit", "0", "--json"])?;
        rows(&listed, "gate", false)?
            .into_iter()
            .filter(gate::awaits_a_pull_request)
            .map(|gate| PrGate::of(&gate, Vec::new()))
            .filter(|gate| wanted(gate))
            .map(|mut gate| {
                let held = self.asked(&[
                    "dep",
                    "list",
                    &gate.id,
                    "--direction=up",
                    "--type",
                    "blocks",
                    "--json",
                ])?;
                gate.blocks = rows(&held, "dep", false)?
                    .into_iter()
                    .map(|bead| bead.id)
                    .collect();
                Ok(gate)
            })
            .collect()
    }
}

impl Tracker for Reader<'_> {
    /// bd over a Dolt server has a probe. bd over its embedded Dolt refuses
    /// it, and that refusal is what tells a tracker with no probe from a
    /// server that did not answer: the second is asked again next refresh,
    /// the first is remembered and never asked again this run.
    fn fingerprint(&self) -> Option<Result<String, RunFailure>> {
        if self.without_a_probe.lock().unwrap().contains(&self.name) {
            return None;
        }
        match self.tables_hashed() {
            Err(failure) if failure.kind == FailureKind::Unsupported => {
                self.without_a_probe
                    .lock()
                    .unwrap()
                    .insert(self.name.clone());
                None
            }
            answer => Some(answer),
        }
    }

    /// One call per project rather than one per root, because a tree is
    /// drawn from dependency edges and `bd dep tree` cannot carry them: it
    /// walks dependents and dedupes, so what comes back is a spanning tree —
    /// each bead with the one edge the walk first reached it by, and every
    /// other edge into it missing. Measured against this project's own
    /// tracker on 2026-08-30, that walk carried 93 of the 176 edges among the
    /// beads it returned. It is also the reason `blocked` is asked for
    /// separately.
    ///
    /// `--all` is load-bearing: without it bd answers about open beads only,
    /// and a smaller correct-looking answer about a different population is
    /// the kind of wrong that reads as right.
    fn all(&self) -> Result<Vec<Bead>, RunFailure> {
        if self.unfinished_work {
            self.every_bead_with_unfinished_text()
        } else {
            self.every_bead()
        }
    }

    /// bd computes readiness itself and treats it as a state of its own, so
    /// it is asked for rather than inferred from status.
    ///
    /// A bare `bd ready` leaves gates out as work nobody claims, and answers
    /// for them by the same rule when asked by type, so gates are asked for
    /// beside it.
    fn ready(&self) -> Result<BTreeSet<String>, RunFailure> {
        let (work, gates) = together(
            || self.asked(&["ready", "--limit", "0", "--json"]),
            || self.asked(&["ready", "--type", "gate", "--limit", "0", "--json"]),
        );
        let mut ready = BTreeSet::new();
        for out in [work?, gates?] {
            ready.extend(rows(&out, "ready", false)?.into_iter().map(|bead| bead.id));
        }
        Ok(ready)
    }

    /// A dep-tree row carries its tree parent, not its blocker set: a bead
    /// blocked by two others appears once, under one of them, with the second
    /// nowhere in the output. `bd blocked` takes no limit of its own.
    fn blocked(&self) -> Result<BTreeMap<String, Vec<String>>, RunFailure> {
        let out = self.asked(&["blocked", "--json"])?;
        let blocked: Vec<BlockedRow> = serde_json::from_str(&out)
            .map_err(|e| RunFailure::parse("bd", e).reading("blocked"))?;
        Ok(blocked
            .into_iter()
            .map(|row| (row.id, row.blocked_by))
            .collect())
    }

    /// `bd events tail` prints its records as JSON lines whether or not it
    /// is given `--json`, and is not given it so that a failure reaches
    /// stderr, where a failure is classified, rather than stdout.
    fn events(&self, since: u64) -> Option<Result<Vec<Value>, RunFailure>> {
        self.journal.then(|| {
            let since = since.to_string();
            let out = self.asked(&["events", "tail", "--since", &since])?;
            records(&out)
        })
    }
}

/// Each line of `bd events tail`, which must carry the `seq` it is read
/// after and the `issue_id` it is routed by.
fn records(out: &str) -> Result<Vec<Value>, RunFailure> {
    out.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let record: Value = serde_json::from_str(line)
                .map_err(|e| RunFailure::parse("bd", e).reading("events"))?;
            if record["seq"].as_u64().is_none() || record["issue_id"].as_str().is_none() {
                return Err(
                    RunFailure::parse("bd", "a record without its seq or issue_id")
                        .reading("events"),
                );
            }
            Ok(record)
        })
        .collect()
}

/// The whole of the SQL `bdi` writes.
///
/// `database()` is the one bd is already connected to. Views are left out
/// because they hold nothing of their own: bd's `ready_issues` and
/// `blocked_issues` are drawn from tables this already hashes.
const TABLE_HASHES: &str = "SELECT table_name AS name, dolt_hashof_table(table_name) AS h \
     FROM information_schema.tables WHERE table_schema = database() \
     AND table_type = 'BASE TABLE' AND table_name <> 'leases' ORDER BY table_name";

/// One row `TABLE_HASHES` answers with.
#[derive(Deserialize)]
struct TableHash {
    name: String,
    h: String,
}

/// The `bd query` expression that selects wisps and nothing else.
const EPHEMERAL: &str = "ephemeral=true";

/// `bd list`, `bd ready` and `bd query` all answer with the same rows, and
/// each names itself so a reader is sent back to the one that broke.
///
/// The root cause rather than the whole chain: `parse_beads` wraps the
/// parser's account in a sentence saying the answer was not understood, which
/// is what the phrase around this already says.
fn rows(out: &str, read: &str, keeping_rows: bool) -> Result<Vec<Bead>, RunFailure> {
    parsed(out, keeping_rows).map_err(|e| RunFailure::parse("bd", e.root_cause()).reading(read))
}

/// The one bd command `bdi bd` passes through, and the first of the two ways
/// `bdi` writes. `Settling` is the second.
const PASSED_THROUGH: [&str; 2] = ["human", "respond"];

/// The flags `bd human respond` takes its response by. Each takes the word
/// after it as that response, whatever the word looks like.
const RESPONSE: [&str; 2] = ["-r", "--response"];

/// bd's argv for `asked`, run against the tracker in `path` and no other.
///
/// A permission rule can limit a seat to `bdi bd <project> …`, and that holds
/// only if nothing later on the line can pick another tracker. bd reads its
/// flags wherever they stand before a `--`, so `-C`, `--db`, `--database` or
/// `--global` anywhere there would outrank the `-C` this puts first. Every
/// flag but the response is refused, so one bd adds later is refused too.
pub fn passed_through(path: &Path, asked: &[String]) -> anyhow::Result<Vec<String>> {
    if asked.len() < PASSED_THROUGH.len() || asked[..PASSED_THROUGH.len()] != PASSED_THROUGH {
        anyhow::bail!(
            "bdi bd passes only `human respond` through to a tracker, and was asked for `{}`",
            asked.join(" ")
        );
    }
    let mut words = asked[PASSED_THROUGH.len()..].iter();
    while let Some(word) = words.next() {
        match word.as_str() {
            "--" => break,
            flag if RESPONSE.contains(&flag) => {
                words.next();
            }
            response if response.starts_with("--response=") => {}
            response if response.starts_with("-r") && !response.starts_with("--") => {}
            flag if flag.starts_with('-') && flag != "-" => anyhow::bail!(
                "bdi bd passes `human respond` its bead and response and nothing else, so it \
                 refuses {flag}; a response that starts with a dash goes after --"
            ),
            _ => {}
        }
    }
    let named = path.to_string_lossy();
    Ok(["-C", named.as_ref()]
        .into_iter()
        .map(String::from)
        .chain(asked.iter().cloned())
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::types::{Dependency, Edge, Status};

    const FIXTURE: &str = include_str!("../../tests/fixtures/bd_list.json");

    fn fixture() -> Vec<Bead> {
        parse_beads(FIXTURE).expect("the captured rows parse")
    }

    fn row(id: &str) -> Bead {
        fixture()
            .into_iter()
            .find(|b| b.id == id)
            .unwrap_or_else(|| panic!("{id} is in the fixture"))
    }

    #[test]
    fn parses_every_row() {
        assert_eq!(fixture().len(), 7);
    }

    const JOINED: &str = include_str!("../../tests/fixtures/joined_bd_list.json");

    /// The bead as `bd show` gives it is in the rows `bd list` already
    /// writes, so showing one costs no further call. Asserted on a capture
    /// rather than a row typed here, because a key a capture carries is a
    /// measurement of what bd writes.
    #[test]
    fn a_captured_row_carries_the_description_the_notes_and_the_name() {
        let rows = parse_beads(JOINED).expect("the captured rows parse");
        let bead = rows
            .iter()
            .find(|b| b.id == "dun-9fw")
            .expect("dun-9fw is in the capture");

        assert!(
            bead.description
                .as_deref()
                .is_some_and(|said| said.starts_with("`dunwich` reads a repository")),
            "{:?}",
            bead.description
        );
        assert!(
            bead.notes
                .as_deref()
                .is_some_and(|said| said.starts_with("Correction to this bead's roster")),
            "{:?}",
            bead.notes
        );
        assert_eq!(bead.created_by.as_deref(), Some("Mira Vance"));
        assert_eq!(bead.assignee.as_deref(), Some("Mira Vance"));
    }

    /// bd leaves both out of a row that has neither, and a row it writes
    /// that way is a bead with nothing to say, not one that will not parse.
    #[test]
    fn a_row_without_a_description_or_notes_parses_with_neither() {
        let bead = row("bdi-2bb.4");

        assert_eq!(bead.description, None);
        assert_eq!(bead.notes, None);
    }

    /// A captured row names every bead it depends on, and the kinds differ
    /// within the one row: the tree cannot be built from the parent edges
    /// alone.
    #[test]
    fn a_captured_row_carries_every_edge_out_of_it() {
        assert_eq!(
            row("bdi-2bb.4").dependencies,
            vec![
                Dependency {
                    on: "bdi-2bb".to_string(),
                    edge: Edge::ParentChild,
                },
                Dependency {
                    on: "bdi-2bb.3".to_string(),
                    edge: Edge::Blocks,
                },
                Dependency {
                    on: "bdi-2bb.9".to_string(),
                    edge: Edge::Blocks,
                },
            ]
        );
    }

    /// `bd dep add` across prefixes, captured on the oldest bd README
    /// supports and on the one this flake pins. Each tracker holds only
    /// `ark-`, and its one row depends on a `dun-` bead held somewhere else.
    const ACROSS_PROJECTS: [(&str, &str, &str); 2] = [
        (
            "1.1.0",
            include_str!("../../tests/fixtures/bd_1.1.0_a_dependency_on_another_project.json"),
            "dun-6hi",
        ),
        (
            "1.3.0",
            include_str!("../../tests/fixtures/bd_1.3.0_a_dependency_on_another_project.json"),
            "dun-2e7",
        ),
    ];

    #[test]
    fn a_dependency_on_another_projects_bead_is_read_as_any_other_edge() {
        for (bd, capture, theirs) in ACROSS_PROJECTS {
            let beads = parse_beads(capture).expect("the captured rows parse");

            assert_eq!(beads.len(), 1, "bd {bd}: the tracker holds only ark's bead");
            assert_eq!(
                beads[0].dependencies,
                vec![Dependency {
                    on: theirs.to_string(),
                    edge: Edge::Blocks,
                }],
                "bd {bd}"
            );
        }
    }

    #[test]
    fn statuses_map_onto_the_enum() {
        assert_eq!(row("bdi-r5l").status, Status::InProgress);
        assert_eq!(row("bdi-2bb").status, Status::Open);
        assert_eq!(row("bdi-2bb.9").status, Status::Closed);
    }

    #[test]
    fn every_status_spelling_bd_writes_is_recognised() {
        let spellings = [
            "open",
            "in_progress",
            "blocked",
            "closed",
            "deferred",
            "pinned",
            "hooked",
        ];
        let expected = [
            Status::Open,
            Status::InProgress,
            Status::Blocked,
            Status::Closed,
            Status::Deferred,
            Status::Pinned,
            Status::Hooked,
        ];

        for (spelling, want) in spellings.iter().zip(expected) {
            let json = format!(r#"[{{"id":"x","title":"t","status":"{spelling}"}}]"#);
            assert_eq!(parse_beads(&json).unwrap()[0].status, want);
        }
    }

    #[test]
    fn a_status_a_later_bd_invents_is_kept_rather_than_rejected() {
        let json = r#"[{"id":"x","title":"t","status":"marinating"}]"#;
        let beads = parse_beads(json).expect("an unknown status still parses");
        assert_eq!(beads[0].status, Status::Other("marinating".to_string()));
    }

    #[test]
    fn an_edge_a_later_bd_invents_is_kept_rather_than_rejected() {
        let json = r#"[{"id":"x","title":"t","status":"open","dependencies":[
          {"depends_on_id":"y","type":"discovered-by"}]}]"#;
        let beads = parse_beads(json).expect("an unknown edge still parses");
        assert_eq!(
            beads[0].dependencies,
            vec![Dependency {
                on: "y".to_string(),
                edge: Edge::Other("discovered-by".to_string()),
            }]
        );
    }

    #[test]
    fn metadata_is_carried_inline_and_absent_metadata_is_an_empty_map() {
        let carrying = row("bdi-r5l");
        assert_eq!(
            carrying.metadata.get("agent_pane").map(String::as_str),
            Some("wCW:p2M")
        );

        assert!(row("bdi-2bb.3").metadata.is_empty());
    }

    /// A row's own fields travel beside its metadata, so a badge can read one
    /// bdi holds no field of its own for.
    #[test]
    fn a_rows_fields_are_carried_under_the_names_bd_spells_them() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open","issue_type":"feature",
             "external_ref":"https://jira.invalid/browse/HELIO-412",
             "metadata":{"jira":"ARKHAM-19","helio.ticket":"HELIO-9"}}
        ]"#;

        let bead = &parse_beads(rows).expect("the row parses")[0];

        assert_eq!(
            bead.value("external_ref"),
            Some("https://jira.invalid/browse/HELIO-412")
        );
        assert_eq!(bead.value("id"), Some("a"));
        assert_eq!(bead.value("issue_type"), Some("feature"));
        assert_eq!(bead.value("metadata.jira"), Some("ARKHAM-19"));
        assert_eq!(
            bead.value("metadata.helio.ticket"),
            Some("HELIO-9"),
            "a key holding a dot of its own is named by the whole of it"
        );
    }

    /// A text `Bead` holds a field of its own for is drawable under bd's name
    /// for it, and is held once: a large tracker's descriptions and notes are
    /// most of what its beads weigh.
    #[test]
    fn a_text_held_in_a_field_is_drawable_and_held_once() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open","issue_type":"task",
             "parent":"p","created_by":"ada","assignee":"grace",
             "description":"what it is","notes":"what was noted"}
        ]"#;

        let bead = &parse_beads(rows).expect("the row parses")[0];

        for (key, text) in [
            ("id", "a"),
            ("title", "t"),
            ("issue_type", "task"),
            ("parent", "p"),
            ("created_by", "ada"),
            ("assignee", "grace"),
            ("description", "what it is"),
            ("notes", "what was noted"),
        ] {
            assert_eq!(bead.value(key), Some(text), "{key} is drawable");
            assert_eq!(bead.values.get(key), None, "{key} is held once");
        }
    }

    /// A text spelling an object is read as that object, as metadata written
    /// before April 2026 is, whether bdi holds a field for the text or not.
    #[test]
    fn a_text_held_in_a_field_that_spells_an_object_is_read_as_its_members() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open",
             "description":"{\"ticket\":\"HELIO-9\"}"}
        ]"#;

        let bead = &parse_beads(rows).expect("the row parses")[0];

        assert_eq!(bead.value("description.ticket"), Some("HELIO-9"));
        assert_eq!(bead.value("description"), None, "an object is no one value");
    }

    /// An empty text is no value to draw, whether bdi holds a field for it
    /// or not.
    #[test]
    fn an_empty_text_held_in_a_field_is_no_value() {
        let rows = r#"[
            {"id":"a","title":"","status":"open","issue_type":"",
             "description":"","notes":"","assignee":"","created_by":""}
        ]"#;

        let bead = &parse_beads(rows).expect("the row parses")[0];

        for absent in [
            "title",
            "issue_type",
            "description",
            "notes",
            "assignee",
            "created_by",
        ] {
            assert_eq!(bead.value(absent), None, "{absent} is no value to draw");
        }
    }

    /// A value is what a badge draws, so a row carries the three kinds that
    /// are one. A null is the field unset and reads as the field being absent,
    /// which is what a badge on an unset field rests on: it would otherwise
    /// draw the four letters `null` on every bead of a tracker nothing syncs.
    /// An array and an object are not one value at all.
    #[test]
    fn a_row_carries_every_value_a_badge_could_draw_and_nothing_that_is_not_one() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open","priority":1,"pinned":true,
             "external_ref":null,"parent":"",
             "dependencies":[{"depends_on_id":"b","type":"blocks"}],
             "metadata":{"jira":"ARKHAM-19"}}
        ]"#;

        let bead = &parse_beads(rows).expect("the row parses")[0];

        assert_eq!(bead.value("priority"), Some("1"));
        assert_eq!(bead.value("pinned"), Some("true"));
        for absent in ["external_ref", "parent", "dependencies", "metadata"] {
            assert_eq!(bead.value(absent), None, "{absent} is no value to draw");
        }
    }

    /// A member of a field's object is judged by what it is, exactly as the
    /// field itself is. The metadata a bead carries is arbitrary JSON, so a
    /// member that is a list or an object of its own is the case this meets,
    /// and rendering one would put its braces on a row beside a title.
    #[test]
    fn a_member_of_an_object_is_one_value_on_the_same_terms_as_a_field() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open",
             "metadata":{"attempts":3,"waiting":false,"phase":"vacuum-soak",
                         "cleared":null,"note":"",
                         "seats":["ada","grace"],"budget":{"hours":4}}}
        ]"#;

        let bead = &parse_beads(rows).expect("the row parses")[0];

        assert_eq!(bead.value("metadata.attempts"), Some("3"));
        assert_eq!(bead.value("metadata.waiting"), Some("false"));
        assert_eq!(bead.value("metadata.phase"), Some("vacuum-soak"));
        for absent in [
            "metadata.cleared",
            "metadata.note",
            "metadata.seats",
            "metadata.budget",
        ] {
            assert_eq!(bead.value(absent), None, "{absent} is no value to draw");
        }
    }

    /// A list is no one value to draw, but a badge's condition can still ask
    /// whether any of its members is the one it wants, so a list of values
    /// is kept a member at a time, a field's and an object member's alike.
    #[test]
    fn a_list_is_read_a_member_at_a_time() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open","labels":["human","rigging"],
             "dependencies":[{"depends_on_id":"b","type":"blocks"}],
             "metadata":{"seats":["ada",3,null,"",{"x":1}],"none":[]}}
        ]"#;

        let bead = &parse_beads(rows).expect("the row parses")[0];

        assert_eq!(bead.members("labels"), vec!["human", "rigging"]);
        assert_eq!(bead.members("metadata.seats"), vec!["ada", "3"]);
        assert_eq!(bead.members("title"), vec!["t"]);
        for nothing in ["dependencies", "metadata.none", "assignee"] {
            assert_eq!(bead.members(nothing), Vec::<&str>::new(), "{nothing}");
        }
        assert_eq!(bead.value("labels"), None, "a list is still no one value");
    }

    /// A tracker's metadata is arbitrary JSON, and bdi draws it as text. A
    /// value that is not a string is read as the text it prints as, because
    /// a tracker is read whole and refusing one value loses every bead in it.
    ///
    /// Measured on a real tracker, 2026-08-31: two beads of 1886 carried
    /// `blocks_backstop_removal: true`, and the whole project failed to read.
    #[test]
    fn a_metadata_value_that_is_not_a_string_is_read_as_its_text() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open",
             "metadata":{"blocks_backstop_removal":true}},
            {"id":"b","title":"t","status":"open",
             "metadata":{"attempts":3,"working_topic":"x"}}
        ]"#;

        let beads = parse_beads(rows).expect("one non-string value does not lose a tracker");

        assert_eq!(
            beads[0].metadata.get("blocks_backstop_removal"),
            Some(&"true".to_string())
        );
        assert_eq!(beads[1].metadata.get("attempts"), Some(&"3".to_string()));
        assert_eq!(
            beads[1].metadata.get("working_topic"),
            Some(&"x".to_string()),
            "a string keeps its own text, without the quotes JSON writes it in"
        );
    }

    /// bd wrote a bead's whole metadata object as a string spelling one until
    /// April 2026, and a tracker old enough to straddle that holds rows of
    /// both shapes.
    #[test]
    fn a_metadata_written_as_a_string_is_read_as_the_object_it_spells() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open","metadata":"{}"},
            {"id":"b","title":"t","status":"open",
             "metadata":"{\"phase\":\"vacuum-soak\",\"attempts\":3}"}
        ]"#;

        let beads = parse_beads(rows).expect("a metadata written as a string still parses");

        assert!(beads[0].metadata.is_empty());
        assert_eq!(
            beads[1].metadata.get("phase"),
            Some(&"vacuum-soak".to_string())
        );
        assert_eq!(
            beads[1].metadata.get("attempts"),
            Some(&"3".to_string()),
            "a value inside the string is read the way one inside an object is"
        );
    }

    /// The other things that string could hold, and the other types the field
    /// could be written as, read as no metadata rather than being refused: a
    /// badge nobody can draw costs one bead, and a refusal costs the project.
    #[test]
    fn a_metadata_that_spells_no_object_is_read_as_none() {
        let rows = r#"[
            {"id":"a","title":"t","status":"open","metadata":"the sails"},
            {"id":"b","title":"t","status":"open","metadata":7}
        ]"#;

        let beads = parse_beads(rows).expect("neither costs the tracker it is in");

        assert!(beads[0].metadata.is_empty());
        assert!(beads[1].metadata.is_empty());
    }

    /// bd omits a field it has nothing for, and `#[serde(default)]` covers
    /// that. It does not extend to an explicit null, which is the same
    /// nothing written the other way.
    #[test]
    fn a_field_written_null_reads_as_the_field_bd_left_out() {
        let rows = r#"[{"id":"a","title":null,"status":"open",
                        "priority":null,"issue_type":null,"metadata":null,
                        "owner":null,"updated_at":null}]"#;

        let beads = parse_beads(rows).expect("a null field does not lose a tracker");

        assert_eq!(beads[0].title, "");
        assert_eq!(beads[0].priority, 0);
        assert_eq!(beads[0].issue_type, "");
        assert!(beads[0].metadata.is_empty());
    }

    /// A null status is not a status bd wrote, so bdi does not put one on the
    /// bead. It reads as a status outside bd's set, which the screen says it
    /// does not recognise — where reading it as `open` would have the bead
    /// claim a status nothing wrote.
    #[test]
    fn a_null_status_is_a_status_bdi_does_not_recognise() {
        let rows = r#"[{"id":"a","title":"t","status":null}]"#;

        let beads = parse_beads(rows).expect("a null status does not lose a tracker");

        assert_eq!(beads[0].status, Status::Other(String::new()));
    }

    /// Leniency about null is not leniency about absence. bd writes a title
    /// and a status on every row, so a listing with neither is a shape bdi
    /// does not understand rather than a row with nothing in those fields.
    #[test]
    fn a_row_that_names_no_title_or_no_status_is_still_an_error() {
        assert!(parse_beads(r#"[{"id":"a","status":"open"}]"#).is_err());
        assert!(parse_beads(r#"[{"id":"a","title":"t"}]"#).is_err());
    }

    #[test]
    fn the_timestamps_the_age_rules_need_follow_the_row() {
        let closed = row("bdi-2bb.9");
        assert!(closed.started_at.is_some());
        assert!(closed.closed_at.is_some());

        let open = row("bdi-2bb");
        assert_eq!(open.started_at, None);
        assert_eq!(open.closed_at, None);
        assert!(open.updated_at.is_some());
    }

    #[test]
    fn an_unclaimed_bead_has_no_assignee() {
        assert_eq!(row("bdi-2bb.9").assignee.as_deref(), Some("Graeme Foster"));
        assert_eq!(row("bdi-2bb").assignee, None);
    }

    #[test]
    fn issue_type_distinguishes_the_root_epic_from_its_tasks() {
        assert_eq!(row("bdi-2bb").issue_type, "epic");
        assert_eq!(row("bdi-2bb.9").issue_type, "task");
    }

    #[test]
    fn a_wrongly_typed_field_is_an_error_not_a_default() {
        let bad = r#"[{"id":"x","title":"t","status":"open","priority":"high"}]"#;
        assert!(parse_beads(bad).is_err());
    }

    #[test]
    fn work_in_flight_ranks_ahead_of_work_that_is_finished() {
        let mut statuses = vec![
            Status::Closed,
            Status::Open,
            Status::Other("marinating".to_string()),
            Status::InProgress,
            Status::Deferred,
            Status::Pinned,
            Status::Blocked,
            Status::Hooked,
        ];
        statuses.sort_by_key(Status::rank);

        assert_eq!(
            statuses,
            vec![
                Status::InProgress,
                Status::Hooked,
                Status::Blocked,
                Status::Open,
                Status::Deferred,
                Status::Pinned,
                Status::Closed,
                Status::Other("marinating".to_string()),
            ]
        );
        assert!(Status::Closed.is_closed());
        assert!(!Status::Open.is_closed());
        assert!(!Status::Pinned.is_closed());
        assert!(!Status::Hooked.is_closed());
    }

    #[test]
    fn a_pinned_bead_is_finished_without_being_closed() {
        assert!(Status::Closed.is_finished());
        assert!(Status::Pinned.is_finished());
        for live in [
            Status::Open,
            Status::InProgress,
            Status::Blocked,
            Status::Deferred,
            Status::Hooked,
            Status::Other("marinating".to_string()),
        ] {
            assert!(!live.is_finished(), "{live:?}");
        }
    }

    use crate::collect::environment::CREDENTIAL_VAR;
    use crate::collect::run::testing::FakeRunner;
    use crate::collect::run::FailureKind;
    use crate::collect::tracker::Trackers;
    use crate::config::{Command, Project};
    use std::path::PathBuf;

    /// A project's directory, named so that nothing is ever there, for the
    /// reason `collect::environment`'s own says: an environment is detected
    /// from what the directory holds, so a path that exists would make these
    /// answer differently on a machine with direnv.
    fn project_dir() -> PathBuf {
        PathBuf::from("/nowhere/a-project")
    }

    /// A bd call as the runner spells it: the tracker named outright, and
    /// writes refused. Every call carries both, so the tests name the
    /// subcommand and this holds the invocation round it.
    fn spelled(subcommand: &str) -> String {
        format!("bd -C {} --readonly {subcommand}", project_dir().display())
    }

    fn credentialled() -> Env {
        Env::from([(CREDENTIAL_VAR.to_string(), "hunter2".to_string())])
    }

    /// The tracker under `project_dir()`, opened on its credential.
    fn opened(runner: &FakeRunner) -> Reader<'_> {
        Reader {
            runner,
            name: "arkham".to_string(),
            path: project_dir(),
            env: credentialled(),
            without_a_probe: Box::leak(Box::default()),
            keeping_rows: false,
            journal: false,
            unfinished_work: false,
        }
    }

    /// A project entry as the config takes it by default: a path and nothing
    /// else, read in `bdi`'s own environment.
    fn ambient_project() -> Project {
        Project {
            name: "arkham".to_string(),
            path: project_dir(),
            environment_command: None,
            credential_command: None,
            prefix: None,
            poll: true,
            events_journal: false,
            badges: Vec::new(),
            worktrees: Vec::new(),
        }
    }

    /// bd's CLI as `bdi` holds it when launched from a shell holding
    /// `ambient`, or from one holding no credential.
    fn launched_with<'a>(runner: &'a FakeRunner, ambient: Option<&str>) -> Cli<'a> {
        Cli {
            runner,
            ambient: ambient.map(str::to_string),
            without_a_probe: Mutex::default(),
            keeping_rows: false,
            reading_journals: false,
            unfinished_work: false,
            cache: None,
        }
    }

    /// The wrapper a direnv setup names, written relative because the command
    /// runs in the project's own directory.
    const DIRENV: &str = "direnv exec .";

    /// The call that reproduces entering `project_dir()`: the configured
    /// wrapper with `bdi`'s own probe appended, through `sh`.
    fn entering_the_directory() -> String {
        format!("{DIRENV} env -0")
    }

    /// The seam's promise: a tracker opened for a project is read in the
    /// environment that project's config asks for, so nothing above the
    /// adapter threads an environment through its calls.
    #[test]
    fn a_tracker_opened_for_a_project_is_read_in_the_environment_its_config_asks_for() {
        let runner = FakeRunner::default()
            .with(
                &entering_the_directory(),
                "BEADS_DOLT_PASSWORD=the-projects-own-password",
            )
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), "[]");
        let project = Project {
            environment_command: Some(Command::Line(DIRENV.to_string())),
            ..ambient_project()
        };

        let cli = launched_with(&runner, Some("the-launching-shells-password"));
        let tracker = cli.of(&project).expect("the directory can be entered");
        tracker.all().expect("the tracker answers");

        let call = runner.call(&spelled(TRACKER_CALL));
        assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
        assert_eq!(
            call.env,
            Env::from([(
                CREDENTIAL_VAR.to_string(),
                "the-projects-own-password".to_string()
            )]),
            "the credential entering the directory produced is the one bd was given"
        );
    }

    /// A project that configures nothing is read on the credential the shell
    /// `bdi` was launched from holds, which the adapter captured once.
    #[test]
    fn a_project_configuring_nothing_is_read_on_the_ambient_credential() {
        let runner = FakeRunner::default()
            .with(&spelled("ready --limit 0 --json"), "[]")
            .with(&spelled("ready --type gate --limit 0 --json"), "[]");

        let cli = launched_with(&runner, Some("hunter2"));
        let tracker = cli
            .of(&ambient_project())
            .expect("nothing is run to open an ambient project");
        tracker.ready().expect("the tracker answers");

        assert_eq!(
            runner.call(&spelled("ready --limit 0 --json")).env,
            credentialled()
        );
    }

    /// Opening is where the environment capture fails, and a project whose
    /// directory cannot be entered is that project's failure before bd is
    /// asked anything — the fake would panic on a bd call nobody staged.
    #[test]
    fn a_project_whose_directory_cannot_be_entered_fails_before_bd_is_asked_anything() {
        let runner = FakeRunner::default().failing(
            &entering_the_directory(),
            RunFailure::unstartable("direnv", "No such file or directory"),
        );
        let project = Project {
            environment_command: Some(Command::Line(DIRENV.to_string())),
            ..ambient_project()
        };

        let failure = launched_with(&runner, None)
            .of(&project)
            .err()
            .expect("the project cannot be opened");

        assert_eq!(failure, OpenFailure::NoEnvironment);
        assert!(
            runner
                .calls()
                .iter()
                .all(|call| !call.argv.starts_with("bd ")),
            "bd was asked something for a project that could not be opened"
        );
    }

    /// The one call a project's whole forest is drawn from, spelled as bd
    /// takes it. `--all` is what makes it the whole tracker rather than its
    /// open beads.
    const TRACKER_CALL: &str = "list --all --include-gates --limit 0 --json";

    /// The second call the same forest needs, because `bd list` answers
    /// about the permanent table only.
    const WISP_CALL: &str = "query ephemeral=true --all --limit 0 --json";

    const WISPS: &str = include_str!("../../tests/fixtures/bd_wisps.json");

    /// The whole invocation the probe makes, spelled out rather than built
    /// from the constant it asserts about: this is the one place `bdi` writes
    /// SQL, and a change to that statement should have to be made twice.
    const PROBE_CALL: &str =
        "sql --json SELECT table_name AS name, dolt_hashof_table(table_name) AS h \
         FROM information_schema.tables WHERE table_schema = database() \
         AND table_type = 'BASE TABLE' AND table_name <> 'leases' ORDER BY table_name";

    /// An answer to the probe from a tracker holding `count` tables, with
    /// the one at `moved` hashing differently from the rest.
    fn tables_hashed(count: usize, moved: Option<usize>) -> String {
        let rows: Vec<String> = (0..count)
            .map(|at| {
                let h = if Some(at) == moved { "b" } else { "a" }.repeat(32);
                format!(r#"{{"name":"table_{at:02}","h":"{h}"}}"#)
            })
            .collect();
        format!("[{}]", rows.join(","))
    }

    /// The fingerprint of a tracker that answers the probe with `answer`.
    fn fingerprint_of(answer: &str) -> String {
        let runner = FakeRunner::default().with(&spelled(PROBE_CALL), answer);
        opened(&runner)
            .fingerprint()
            .expect("bd has a probe")
            .expect("the tracker answered its table hashes")
    }

    #[test]
    fn the_fingerprint_is_every_tables_hash_out_of_one_statement() {
        let runner = FakeRunner::default().with(&spelled(PROBE_CALL), &tables_hashed(2, None));

        let answered = opened(&runner)
            .fingerprint()
            .expect("bd has a probe")
            .expect("the tracker answered its table hashes");

        assert_eq!(
            answered,
            fingerprint_of(&tables_hashed(2, None)),
            "a tracker that has not moved answers the same fingerprint"
        );
        assert_eq!(
            runner.call(&spelled(PROBE_CALL)).env,
            credentialled(),
            "the probe reaches the tracker on the project's own credential"
        );
    }

    /// Dolt cuts a `GROUP_CONCAT` at `group_concat_max_len`, 1024 bytes by
    /// default, and ignores a `SET_VAR` hint raising it, so the hashes come
    /// back a row each and are folded here. Forty tables is past where that
    /// cut would fall, and the last of them still moves the fingerprint.
    #[test]
    fn a_move_in_any_one_table_moves_the_fingerprint_however_many_there_are() {
        let unmoved = fingerprint_of(&tables_hashed(40, None));

        for moved in [0, 20, 39] {
            assert_ne!(
                fingerprint_of(&tables_hashed(40, Some(moved))),
                unmoved,
                "table {moved} of 40 moved and the fingerprint did not"
            );
        }
    }

    /// bd's refusal of the probe on a store that cannot run it, as the runner
    /// classifies it.
    fn cannot_run_the_probe() -> RunFailure {
        RunFailure {
            kind: FailureKind::Unsupported,
            program: "bd".to_string(),
            detail: "bd cannot run that against this tracker".to_string(),
            unreadable: None,
        }
    }

    /// A Dolt server that did not answer the probe, as the runner classifies
    /// it.
    fn did_not_answer_the_probe() -> RunFailure {
        RunFailure {
            kind: FailureKind::Unavailable,
            program: "bd".to_string(),
            detail: "bd could not reach the tracker".to_string(),
            unreadable: None,
        }
    }

    /// How many times the probe was run against `project_dir()`.
    fn probes(runner: &FakeRunner) -> usize {
        runner
            .calls()
            .iter()
            .filter(|call| call.argv == spelled(PROBE_CALL))
            .count()
    }

    /// bd's default store is its embedded Dolt, which refuses `bd sql`, and
    /// the refusal is the same on every refresh. So a tracker found to have
    /// no probe is read in full from then on without the probe being paid
    /// for again: one failing bd process per project per run, not per
    /// refresh.
    #[test]
    fn a_tracker_found_to_have_no_probe_is_not_probed_again_that_run() {
        let runner = FakeRunner::default().failing(&spelled(PROBE_CALL), cannot_run_the_probe());
        let cli = launched_with(&runner, None);
        let project = ambient_project();

        let first = cli.of(&project).expect("opened").fingerprint();
        let second = cli.of(&project).expect("opened").fingerprint();

        assert!(
            first.is_none(),
            "the refusal is a tracker with no probe: {first:?}"
        );
        assert!(second.is_none(), "and stays one: {second:?}");
        assert_eq!(probes(&runner), 1, "the probe was paid for once");
    }

    /// A Dolt server that is down answers the probe with nothing, and comes
    /// back. That is a probe worth asking again, and it is not remembered:
    /// a run that took an outage for "no probe" would never take the fast
    /// path again once the server was up.
    #[test]
    fn a_server_that_did_not_answer_the_probe_is_probed_again_on_the_next_refresh() {
        let runner =
            FakeRunner::default().failing(&spelled(PROBE_CALL), did_not_answer_the_probe());
        let cli = launched_with(&runner, None);
        let project = ambient_project();

        for refresh in 1..=2 {
            let failure = cli
                .of(&project)
                .expect("opened")
                .fingerprint()
                .expect("a server has a probe")
                .expect_err("the server did not answer");
            assert_eq!(failure.kind, FailureKind::Unavailable);
            assert_eq!(probes(&runner), refresh, "one probe per refresh");
        }
    }

    /// What is remembered is which tracker has no probe, not that the run
    /// has stopped probing: a second project on a Dolt server is probed as
    /// ever alongside one that refused.
    #[test]
    fn no_probe_is_remembered_for_the_tracker_that_refused_and_not_its_neighbours() {
        let harbour = Project {
            name: "harbour".to_string(),
            path: PathBuf::from("/tmp/harbour"),
            ..ambient_project()
        };
        let harbours_probe = format!("bd -C {} --readonly {PROBE_CALL}", harbour.path.display());
        let runner = FakeRunner::default()
            .failing(&spelled(PROBE_CALL), cannot_run_the_probe())
            .with(&harbours_probe, &tables_hashed(2, None));
        let cli = launched_with(&runner, None);

        assert!(cli
            .of(&ambient_project())
            .expect("opened")
            .fingerprint()
            .is_none());
        let root = cli
            .of(&harbour)
            .expect("opened")
            .fingerprint()
            .expect("harbour's server has a probe")
            .expect("and answered it");

        assert_eq!(root, fingerprint_of(&tables_hashed(2, None)));
    }

    /// An answer with no row is a tracker that cannot be compared against,
    /// not a tracker that has not moved — so it fails rather than answering
    /// something a caller would gate on.
    #[test]
    fn a_probe_that_answers_no_row_is_a_failure_rather_than_a_hash() {
        let runner = FakeRunner::default().with(&spelled(PROBE_CALL), "[]");

        let failure = opened(&runner)
            .fingerprint()
            .expect("bd has a probe")
            .expect_err("no row is no answer");

        assert_eq!(failure.kind, FailureKind::Parse);
    }

    /// A tracker with no `dolt_hashof_table` — a SQLite-backed one — answers
    /// with something this cannot read, and that is a failure the caller has
    /// to see rather than a hash it would compare against.
    #[test]
    fn a_probe_answering_something_else_is_a_failure_rather_than_a_hash() {
        let runner = FakeRunner::default().with(&spelled(PROBE_CALL), "no such function");

        let failure = opened(&runner)
            .fingerprint()
            .expect("bd has a probe")
            .expect_err("an answer that is not the row is no answer");

        assert_eq!(failure.kind, FailureKind::Parse);
    }

    #[test]
    fn the_tracker_is_read_in_the_projects_directory_with_its_credential() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), "[]");

        let beads = opened(&runner).all().unwrap();

        assert_eq!(beads.len(), 7);
        for subcommand in [TRACKER_CALL, WISP_CALL] {
            let call = runner.call(&spelled(subcommand));
            assert_eq!(call.cwd.as_deref(), Some(project_dir().as_path()));
            assert_eq!(call.env, credentialled());
        }
    }

    /// A reader keeping rows hands each bead over with its row exactly as bd
    /// printed it, so a field `bdi` holds nothing of reaches whoever reads
    /// the row.
    #[test]
    fn a_reader_keeping_rows_hands_each_bead_over_with_the_row_bd_printed() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), WISPS);
        let printed: Vec<serde_json::Map<String, serde_json::Value>> = [FIXTURE, WISPS]
            .iter()
            .flat_map(|out| serde_json::from_str::<Vec<_>>(out).expect("the capture parses"))
            .collect();

        let beads = Reader {
            keeping_rows: true,
            ..opened(&runner)
        }
        .all()
        .unwrap();

        let rows: Vec<_> = beads
            .iter()
            .map(|bead| bead.row.as_deref().cloned().expect("the row is kept"))
            .collect();
        assert_eq!(rows, printed);
    }

    /// A comment, a dependency added, the update bd writes for the bead that
    /// dependency blocks, and a close, from a throwaway bd 1.3.0 tracker.
    const JOURNAL: &str = include_str!("../../tests/fixtures/bd_1.3.0_events_tail.jsonl");

    #[test]
    fn a_journal_is_read_after_the_seq_given_and_each_record_kept_as_bd_printed_it() {
        let runner = FakeRunner::default().with(&spelled("events tail --since 2"), JOURNAL);
        let printed: Vec<Value> = JOURNAL
            .lines()
            .map(|line| serde_json::from_str(line).expect("the capture parses"))
            .collect();

        let records = Reader {
            journal: true,
            ..opened(&runner)
        }
        .events(2)
        .expect("the journal is read")
        .expect("bd answers");

        assert_eq!(records, printed);
        assert_eq!(
            records.iter().map(|r| r["op"].as_str()).collect::<Vec<_>>(),
            [
                Some("comment"),
                Some("dep_add"),
                Some("update"),
                Some("close")
            ]
        );
    }

    /// The fake runner panics on any call nobody staged.
    #[test]
    fn a_tracker_opened_without_its_journal_asks_bd_nothing_of_it() {
        assert_eq!(opened(&FakeRunner::default()).events(0), None);
    }

    #[test]
    fn only_a_project_claiming_a_journal_has_it_read() {
        let runner = FakeRunner::default().with(&spelled("events tail --since 0"), JOURNAL);
        let claiming = Project {
            events_journal: true,
            ..ambient_project()
        };
        let reading = launched_with(&runner, Some("hunter2")).reading_journals();
        let not_reading = launched_with(&runner, Some("hunter2"));

        let events =
            |cli: &Cli, project: &Project| cli.of(project).expect("the tracker opens").events(0);

        assert!(matches!(events(&reading, &claiming), Some(Ok(_))));
        assert_eq!(events(&reading, &ambient_project()), None);
        assert_eq!(events(&not_reading, &claiming), None);
    }

    #[test]
    fn a_record_without_its_seq_is_a_journal_bdi_cannot_read() {
        let runner = FakeRunner::default().with(
            &spelled("events tail --since 0"),
            r#"{"op":"close","issue_id":"dun-1"}"#,
        );

        let failure = Reader {
            journal: true,
            ..opened(&runner)
        }
        .events(0)
        .expect("the journal is read")
        .expect_err("a record without its seq cannot be read after");

        assert_eq!(failure.kind, FailureKind::Parse);
        assert_eq!(failure.unreadable.expect("a parse failure").read, "events");
    }

    #[test]
    fn a_reader_not_keeping_rows_holds_none() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), WISPS);

        let beads = opened(&runner).all().unwrap();

        assert!(beads.iter().all(|bead| bead.row.is_none()));
    }

    /// `bd list` answers about the permanent table, so it returns no wisp at
    /// all — not under `--all`, and not under its own `--wisp-type` filter.
    /// A tracker read only that way draws none of them.
    #[test]
    fn a_tracker_answers_with_its_wisps_as_well_as_its_permanent_beads() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), WISPS);

        let beads = opened(&runner).all().unwrap();

        let ids: Vec<&str> = beads.iter().map(|bead| bead.id.as_str()).collect();
        assert!(
            ids.contains(&"bdi-7ao.17.2"),
            "the wisp under a bead: {ids:?}"
        );
        assert!(
            ids.contains(&"bdi-wisp-w3m"),
            "the free-standing wisp: {ids:?}"
        );
        assert_eq!(beads.len(), 9, "both answers, neither replacing the other");
    }

    /// What a reader for unfinished work asks for in place of the whole
    /// listing: the unfinished beads whole, and every bead without its free
    /// text.
    const UNFINISHED_CALL: &str = "list --include-gates --limit 0 --json";
    const BRIEF_CALL: &str = "list --all --include-gates --brief --limit 0 --json";

    fn reading_unfinished_work(runner: &FakeRunner) -> Reader<'_> {
        Reader {
            unfinished_work: true,
            ..opened(runner)
        }
    }

    /// ark-1.1 is the one unfinished bead, under a closed epic.
    const ARK_UNFINISHED: &str = r#"[{"id":"ark-1.1","title":"chart the reef","status":"open",
        "priority":2,"issue_type":"task","description":"from the lighthouse to the point",
        "parent":"ark-1","dependencies":[{"depends_on_id":"ark-1","type":"parent-child"}]}]"#;

    /// Every bead in the same tracker as a brief listing writes it, with no
    /// description on any of them.
    const ARK_BRIEFLY: &str = r#"[
        {"id":"ark-1","title":"survey the harbour","status":"closed","priority":1,"issue_type":"epic"},
        {"id":"ark-1.1","title":"chart the reef","status":"open","priority":2,"issue_type":"task",
         "parent":"ark-1","dependencies":[{"depends_on_id":"ark-1","type":"parent-child"}]}
    ]"#;

    #[test]
    fn a_read_for_unfinished_work_takes_unfinished_beads_whole_and_the_rest_briefly() {
        let runner = FakeRunner::default()
            .with(&spelled(UNFINISHED_CALL), ARK_UNFINISHED)
            .with(&spelled(BRIEF_CALL), ARK_BRIEFLY)
            .with(&spelled(WISP_CALL), WISPS);

        let beads = reading_unfinished_work(&runner).all().unwrap();

        let described: Vec<(&str, Option<&str>)> = beads
            .iter()
            .map(|bead| (bead.id.as_str(), bead.description.as_deref()))
            .filter(|(id, _)| id.starts_with("ark-"))
            .collect();
        assert_eq!(
            described,
            [
                ("ark-1.1", Some("from the lighthouse to the point")),
                ("ark-1", None)
            ]
        );
        assert_eq!(beads.len(), 4, "the two beads once each, and both wisps");
    }

    #[test]
    fn a_read_for_unfinished_work_asks_for_all_three_listings_together() {
        let runner = FakeRunner::default()
            .with(&spelled(UNFINISHED_CALL), ARK_UNFINISHED)
            .with(&spelled(BRIEF_CALL), ARK_BRIEFLY)
            .with(&spelled(WISP_CALL), "[]")
            .meeting(&[
                &spelled(UNFINISHED_CALL),
                &spelled(BRIEF_CALL),
                &spelled(WISP_CALL),
            ]);

        reading_unfinished_work(&runner).all().unwrap();

        assert_eq!(runner.waited_alone(), Vec::<String>::new());
    }

    /// bd's default listing leaves out a status bd counts as done, which
    /// `bdi` draws as unfinished, so that bead would be shown without its
    /// text. The tracker is read whole instead.
    #[test]
    fn a_tracker_holding_a_status_bd_counts_as_done_is_read_whole() {
        let shelved = r#"[{"id":"ark-3","title":"paint the hull","status":"shelved",
            "priority":2,"issue_type":"task"}]"#;
        let runner = FakeRunner::default()
            .with(&spelled(UNFINISHED_CALL), "[]")
            .with(&spelled(BRIEF_CALL), shelved)
            .with(&spelled(WISP_CALL), "[]")
            .with(&spelled(TRACKER_CALL), FIXTURE);

        let beads = reading_unfinished_work(&runner).all().unwrap();

        assert_eq!(beads.len(), fixture().len());
    }

    /// A project can pin a bd older than `--brief`, which bd 1.2.0 added.
    #[test]
    fn a_tracker_whose_bd_has_no_brief_listing_is_read_whole() {
        let runner = FakeRunner::default()
            .with(&spelled(UNFINISHED_CALL), "[]")
            .failing(
                &spelled(BRIEF_CALL),
                RunFailure {
                    kind: FailureKind::UnknownFlag,
                    program: "bd".to_string(),
                    detail: "bd does not know a flag bdi uses".to_string(),
                    unreadable: None,
                },
            )
            .with(&spelled(WISP_CALL), "[]")
            .with(&spelled(TRACKER_CALL), FIXTURE);

        let beads = reading_unfinished_work(&runner).all().unwrap();

        assert_eq!(beads.len(), fixture().len());
    }

    /// The listing and the wisps are two round trips that need nothing from
    /// each other, so a read pays for the slower of them rather than both.
    #[test]
    fn the_listing_and_the_wisps_are_asked_for_together() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), WISPS)
            .meeting(&[&spelled(TRACKER_CALL), &spelled(WISP_CALL)]);

        opened(&runner).all().unwrap();

        assert_eq!(runner.waited_alone(), Vec::<String>::new());
    }

    /// bd omits a field it has nothing for rather than writing it as null:
    /// measured across the 73 ephemeral rows of a tracker on 2026-08-31,
    /// eleven keys are universal and every other one is absent when empty.
    /// So this row is not a shape bd writes today. It is covered because
    /// `#[serde(default)]` does not extend to an explicit null, and a
    /// tracker is read whole — a single row bd wrote differently would cost
    /// every bead in that project rather than its own edges.
    #[test]
    fn a_row_naming_its_dependencies_as_null_still_parses() {
        let json = r#"[{"id":"nix-wisp-gvi","title":"t","status":"open",
                        "issue_type":"molecule","parent":null,"dependencies":null}]"#;

        let bead = &parse_beads(json).expect("the row parses")[0];

        assert_eq!(bead.dependencies, vec![]);
    }

    /// A wisp bd writes under a parent is a child like any other: a dotted id
    /// and a real parent-child edge. One written without a parent has neither,
    /// so nothing but root discovery can place it.
    #[test]
    fn a_wisp_carries_the_edge_that_hangs_it_under_a_bead_where_it_has_one() {
        let wisps = parse_beads(WISPS).expect("the captured wisps parse");
        let by_id = |id: &str| {
            wisps
                .iter()
                .find(|wisp| wisp.id == id)
                .unwrap_or_else(|| panic!("{id} is in the fixture"))
                .clone()
        };

        assert_eq!(
            by_id("bdi-7ao.17.2").dependencies,
            vec![Dependency {
                on: "bdi-7ao.17".to_string(),
                edge: Edge::ParentChild,
            }]
        );
        assert_eq!(by_id("bdi-wisp-w3m").dependencies, vec![]);
    }

    /// One completed molecule and its sixteen steps, from a real wisp run on
    /// another project's tracker. It is here as evidence of what bd writes,
    /// which a constant somebody typed cannot be: the author of a constant
    /// supplies the keys they remember, so a key absent from a capture is a
    /// measurement and a key absent from a constant is authorship.
    ///
    /// Its two-row neighbour is not replaced by it. That one carries a
    /// free-standing wisp under no parent, which a molecule run has none of.
    ///
    /// Every `title` in it is invented, and says so in its own value. The
    /// capture arrived without the key: the redaction dropped `title`,
    /// `description`, `notes`, `owner`, `created_by`, `assignee`,
    /// `close_reason` and `dependencies[].created_by` whole, because one
    /// close reason named a person. `Bead::title` is required, so the file
    /// could not be parsed without one. Nothing else was touched — key order
    /// is the capture's and no value it carried was rewritten. Assert on a
    /// title and you are asserting on something we made up.
    const MOLECULE: &str = include_str!("../../tests/fixtures/bd_wisp_molecule.json");

    /// A row bd writes carries keys bdi does not read, and the ones bd will
    /// write next are not knowable. `await_type`, on the gate step of this
    /// run, is one nothing else in the tree has — which is the point of
    /// keeping a capture rather than a constant, because a constant carries
    /// only the keys its author already knew about.
    ///
    /// That `Bead` ignores such a key rather than rejecting the row is held
    /// by most of this module already. What is held here is the evidence:
    /// tidying the capture down to the fields bdi reads is what this refuses,
    /// because the untidy keys are the measurement.
    ///
    /// The counts go with it so a file shortened by accident is noticed. They
    /// are not evidence of anything — a run of any length can be typed.
    #[test]
    fn the_capture_keeps_a_field_bdi_does_not_read() {
        let run = parse_beads(MOLECULE).expect("the captured molecule parses");

        assert!(
            MOLECULE.contains(r#""await_type""#),
            "the capture still carries the key this is about"
        );
        assert_eq!(run.len(), 17, "the molecule and its steps");
        assert_eq!(
            run.iter()
                .map(|bead| bead.dependencies.len())
                .sum::<usize>(),
            37,
            "the edges bd wrote between them"
        );
        assert_eq!(
            run.iter()
                .filter(|bead| bead.issue_type == "molecule")
                .count(),
            1
        );
    }

    /// bd omits a field it has nothing for rather than writing it as null,
    /// which is what lets every field bdi reads carry `#[serde(default)]`.
    /// That is a claim about a program we do not own, so this holds the
    /// evidence for it: seventeen rows and their thirty-seven edges as bd
    /// wrote them, with no null anywhere — including on the molecule, which
    /// omits `parent` and `dependencies` rather than nulling them.
    ///
    /// It goes red if a later bd starts writing nulls, which is the day
    /// `none_is_empty` stops being enough.
    #[test]
    fn bd_omits_what_it_has_nothing_for_rather_than_writing_null() {
        let rows: serde_json::Value = serde_json::from_str(MOLECULE).expect("the capture is json");

        fn nulls(value: &serde_json::Value, at: &str, found: &mut Vec<String>) {
            match value {
                serde_json::Value::Null => found.push(at.to_string()),
                serde_json::Value::Object(fields) => {
                    for (key, field) in fields {
                        nulls(field, &format!("{at}.{key}"), found);
                    }
                }
                serde_json::Value::Array(items) => {
                    for (i, item) in items.iter().enumerate() {
                        nulls(item, &format!("{at}[{i}]"), found);
                    }
                }
                _ => {}
            }
        }

        let mut found = Vec::new();
        nulls(&rows, "", &mut found);

        assert_eq!(found, Vec::<String>::new(), "nulls bd wrote");
    }

    #[test]
    fn a_row_carries_every_bead_it_depends_on_and_the_kind_of_each() {
        let json = r#"[{"id":"p-1.4","title":"t","status":"open","dependencies":[
          {"issue_id":"p-1.4","depends_on_id":"p-1","type":"parent-child"},
          {"issue_id":"p-1.4","depends_on_id":"p-1.3","type":"blocks"}]}]"#;
        let bead = &parse_beads(json).expect("the row parses")[0];

        assert_eq!(
            bead.dependencies,
            vec![
                Dependency {
                    on: "p-1".to_string(),
                    edge: Edge::ParentChild,
                },
                Dependency {
                    on: "p-1.3".to_string(),
                    edge: Edge::Blocks,
                },
            ]
        );
    }

    #[test]
    fn ready_ids_returns_the_set_bd_considers_startable() {
        let out = r#"[{"id":"p-1.1","title":"a","status":"open"},
                      {"id":"p-1.3","title":"b","status":"open"}]"#;
        let runner = FakeRunner::default()
            .with(&spelled("ready --limit 0 --json"), out)
            .with(&spelled("ready --type gate --limit 0 --json"), "[]");

        let got = opened(&runner).ready().unwrap();

        assert!(got.contains("p-1.1"));
        assert!(got.contains("p-1.3"));
        assert!(
            !got.contains("p-1.4"),
            "a bead bd did not list is not ready"
        );
    }

    /// A bare `bd ready` leaves gates out as work nobody claims, and lists
    /// them when asked for by type, by the same rule as any other bead.
    #[test]
    fn a_gate_bd_lists_when_asked_for_gates_is_ready() {
        let runner = FakeRunner::default()
            .with(
                &spelled("ready --limit 0 --json"),
                r#"[{"id":"p-1.1","title":"a","status":"open"}]"#,
            )
            .with(
                &spelled("ready --type gate --limit 0 --json"),
                r#"[{"id":"p-wg1","title":"Gate: gh:pr","status":"open","issue_type":"gate"}]"#,
            );

        let got = opened(&runner).ready().unwrap();

        assert_eq!(got, BTreeSet::from(["p-1.1".into(), "p-wg1".into()]));
    }

    /// The shape a real tracker produces: a bead blocked by two beads, whose
    /// dep-tree row names only one of them.
    #[test]
    fn blocked_by_carries_every_blocker_not_only_the_one_the_tree_shows() {
        let out = r#"[{"id":"p-1.9","title":"a","status":"blocked","blocked_by_count":2,
                       "blocked_by":["p-1.2","p-1.5"]},
                      {"id":"p-1.11","title":"b","status":"open","blocked_by_count":1,
                       "blocked_by":["p-1.10"]}]"#;
        let runner = FakeRunner::default().with(&spelled("blocked --json"), out);

        let got = opened(&runner).blocked().unwrap();

        assert_eq!(
            got.get("p-1.9").map(Vec::as_slice),
            Some(["p-1.2".to_string(), "p-1.5".to_string()].as_slice())
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got.get("p-1.1"), None);
    }

    /// A tracker is read whole, so a `blocked_by` in a shape bdi does not
    /// expect would otherwise cost every row of the listing. The row keeps
    /// the ids it can read, and every other row is untouched.
    #[test]
    fn a_blocked_by_in_an_unexpected_shape_keeps_the_rows_it_can_read() {
        let out = r#"[{"id":"p-1.2","blocked_by":null},
                      {"id":"p-1.3","blocked_by":"p-1.1"},
                      {"id":"p-1.4","blocked_by":{"id":"p-1.1"}},
                      {"id":"p-1.5","blocked_by":["p-1.1",7,null,"p-1.9"]},
                      {"id":"p-1.9","blocked_by":["p-1.10"]}]"#;
        let runner = FakeRunner::default().with(&spelled("blocked --json"), out);

        let got = opened(&runner)
            .blocked()
            .expect("an unexpected blocked_by does not lose the listing");

        let none: &[String] = &[];
        assert_eq!(got.get("p-1.2").map(Vec::as_slice), Some(none));
        assert_eq!(got.get("p-1.3").map(Vec::as_slice), Some(none));
        assert_eq!(got.get("p-1.4").map(Vec::as_slice), Some(none));
        assert_eq!(
            got.get("p-1.5").map(Vec::as_slice),
            Some(["p-1.1".to_string(), "p-1.9".to_string()].as_slice())
        );
        assert_eq!(
            got.get("p-1.9").map(Vec::as_slice),
            Some(["p-1.10".to_string()].as_slice())
        );
    }

    #[test]
    fn a_tracker_that_refuses_the_credential_reaches_the_caller_classified() {
        let runner = FakeRunner::default()
            .failing(
                &spelled(TRACKER_CALL),
                RunFailure {
                    kind: FailureKind::Auth,
                    program: "bd".to_string(),
                    detail: "bd was refused the tracker's credential".to_string(),
                    unreadable: None,
                },
            )
            .with(&spelled(WISP_CALL), "[]");

        let failure = opened(&runner).all().unwrap_err();

        assert_eq!(failure.kind, FailureKind::Auth);
    }

    /// Which read broke and where in its answer, because that is the whole
    /// of what a reader can do about one: run that read themselves and go to
    /// the row the parser stopped at. The kind alone sends them to a tracker
    /// with five reads in it and no way to tell which.
    #[test]
    fn a_listing_that_will_not_parse_names_the_read_and_where_it_broke() {
        let row = r#"[{"id":"ark-1","title":42,"status":"open"}]"#;
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), row)
            .with(&spelled(WISP_CALL), "[]");

        let unreadable = opened(&runner)
            .all()
            .unwrap_err()
            .unreadable
            .expect("a parse failure knows what would not parse");

        assert_eq!(unreadable.read, "list");
        assert_eq!(
            unreadable.cause,
            "invalid type: integer `42`, expected a string at line 1 column 25"
        );
    }

    /// The wisps are a second read of the same rows, and a reader sent to
    /// `bd list` for a row `bd query` answered with looks at an answer that
    /// holds no such row.
    #[test]
    fn a_wisp_that_will_not_parse_names_the_read_that_carried_it() {
        let row = r#"[{"id":"ark-2","title":42,"status":"open"}]"#;
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), "[]")
            .with(&spelled(WISP_CALL), row);

        let unreadable = opened(&runner)
            .all()
            .unwrap_err()
            .unreadable
            .expect("a parse failure knows what would not parse");

        assert_eq!(unreadable.read, "query");
    }

    /// Bytes that are not UTF-8 refuse before any row is looked at, so the
    /// read is named by the call that composed the command line rather than
    /// by the parser.
    #[test]
    fn an_answer_that_is_not_text_names_the_read_it_came_from() {
        let runner = FakeRunner::default().failing(
            &spelled("blocked --json"),
            RunFailure::parse("bd", "invalid utf-8 sequence of 1 bytes from index 3"),
        );

        let unreadable = opened(&runner)
            .blocked()
            .unwrap_err()
            .unreadable
            .expect("a parse failure knows what would not parse");

        assert_eq!(unreadable.read, "blocked");
    }

    #[test]
    fn output_bd_could_not_have_written_is_a_parse_failure_not_an_unreachable_tracker() {
        let runner = FakeRunner::default().with(&spelled("blocked --json"), "not json at all");

        let failure = opened(&runner).blocked().unwrap_err();

        assert_eq!(failure.kind, FailureKind::Parse);
    }

    /// The whole of the invocation, in one test because the two halves are
    /// one fact: bd is told which tracker to read, and told it may not write
    /// to it. Asserting the tracker alone would pass with the writes still
    /// allowed.
    #[test]
    fn every_call_names_the_tracker_outright_and_refuses_writes() {
        let runner = FakeRunner::default()
            .with(&spelled(TRACKER_CALL), FIXTURE)
            .with(&spelled(WISP_CALL), "[]");

        opened(&runner).all().unwrap();

        for call in runner.calls() {
            let after_the_program = call
                .argv
                .strip_prefix("bd ")
                .unwrap_or_else(|| panic!("{} is not a bd call", call.argv));
            assert!(
                after_the_program
                    .starts_with(&format!("-C {} --readonly ", project_dir().display())),
                "the tracker is left to the working directory in: {}",
                call.argv
            );
        }
    }

    /// A captured row carries the bead's own parent, which is the one the
    /// walk to a root needs: a dep-tree row's `parent_id` is the traversal's.
    #[test]
    fn a_captured_row_carries_the_bead_it_hangs_under() {
        assert_eq!(row("bdi-2bb.4").parent.as_deref(), Some("bdi-2bb"));
        assert_eq!(row("bdi-2bb").parent.as_deref(), Some("bdi-7ao"));
    }

    /// bd writes a root's absent parent as `null`; the dep tree writes the
    /// same absence as `""`, and an older bd omitted the field. All three
    /// mean the same thing.
    #[test]
    fn a_root_has_no_parent_however_bd_spells_the_absence() {
        for spelling in [r#","parent":null"#, r#","parent":"""#, ""] {
            let out = format!(r#"[{{"id":"p-1","title":"a","status":"open"{spelling}}}]"#);

            assert_eq!(
                parse_beads(&out).expect("the row parses")[0].parent,
                None,
                "on {spelling:?}"
            );
        }
    }

    // ---- the one command line passed through --------------------------------

    fn words(line: &str) -> Vec<String> {
        line.split_whitespace().map(String::from).collect()
    }

    fn passed(line: &str) -> anyhow::Result<String> {
        passed_through(&project_dir(), &words(line)).map(|argv| argv.join(" "))
    }

    /// The tracker is named outright and first, and nothing refuses the
    /// write: `--readonly` is what every read carries, and it would veto this.
    #[test]
    fn a_response_is_passed_to_the_projects_tracker_as_written() {
        assert_eq!(
            passed("human respond dun-7 -r yes").expect("a response is passed through"),
            format!("-C {} human respond dun-7 -r yes", project_dir().display())
        );
    }

    #[test]
    fn every_way_bd_takes_a_response_by_flag_or_word_is_passed_through() {
        for line in [
            "human respond dun-7 use OAuth2",
            "human respond dun-7 --response yes",
            "human respond dun-7 --response=yes",
            "human respond dun-7 -ryes",
            "human respond dun-7 -- -C is not a flag here",
        ] {
            assert!(passed(line).is_ok(), "{line:?} was refused");
        }
    }

    #[test]
    fn any_other_bd_command_is_refused() {
        for line in [
            "close dun-7",
            "human dismiss dun-7",
            "human",
            "respond dun-7",
        ] {
            let refused = passed(line).expect_err(line);

            assert!(
                refused.to_string().contains("human respond"),
                "{line:?} got: {refused}"
            );
        }
    }

    /// Every spelling cobra reads a tracker's flag by, including a shorthand
    /// bundled behind another.
    #[test]
    fn a_flag_that_would_pick_another_tracker_is_refused_wherever_it_stands() {
        for line in [
            "human respond dun-7 -C /srv/work/ferry yes",
            "human respond dun-7 yes --db /srv/work/ferry/.beads",
            "human respond dun-7 --db=ferry yes",
            "human respond dun-7 --database ferry yes",
            "human respond dun-7 --directory=/srv/work/ferry yes",
            "human respond dun-7 --global yes",
            "human respond dun-7 -qC /srv/work/ferry yes",
            "human respond dun-7 -r yes --db ferry",
            "human respond dun-7 -r -- --db ferry",
            "--db ferry human respond dun-7 yes",
        ] {
            assert!(passed(line).is_err(), "{line:?} was passed through");
        }
    }

    #[test]
    fn a_flag_that_is_not_the_response_is_refused_and_named() {
        for flag in ["--json", "--stdin", "--file", "-q", "--actor"] {
            let line = format!("human respond dun-7 {flag} x");
            let refused = passed(&line).expect_err(&line);

            assert!(refused.to_string().contains(flag), "got: {refused}");
        }
    }

    /// The value after `-r` is the response whatever it looks like, which is
    /// how cobra reads it too.
    #[test]
    fn a_response_that_looks_like_a_flag_is_still_a_response() {
        assert!(passed("human respond dun-7 -r --global").is_ok());
        assert!(passed("human respond dun-7 --response -C").is_ok());
    }
}
