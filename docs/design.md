# beady-eye — design

Status: describes `bdi` as built, reconciled against `main` on 2026-09-02
Date: 2026-08-30, reconciled 2026-09-02
Binary: `bdi`

This was the starting point, not gospel. Where it was ambiguous or wrong, the
seat that hit the ambiguity decided it against the code in front of it and
recorded what it chose; those decisions are folded back in here. Where a later
change makes a sentence here false, the code is right and this document is the
one to fix.

**A measurement names whatever can falsify it.** A date alone dates the
reading, not the thing read. A claim about `bdi`'s own behaviour names the
commit it was measured at; a claim about `bd`, direnv or another tool names
that tool's version, because a `bdi` sha says nothing about it; a claim that
turns on this repository's own `.envrc` says so. Nothing checks this — `nix
flake check` compiles code and nothing reads a claim — so it is a convention
for the writer, and the reader's cue that a figure with no such stamp is a
figure nobody can retire.

## The problem

Work is tracked in [beads](https://github.com/steveyegge/beads) and done by
coding agents running in terminal panes. Two systems hold the truth and neither
holds all of it:

- **bd** knows the work — the tree of beads, the dependency edges, each bead's
  status and who claimed it.
- **herdr** knows the agents — which pane is alive and what it is doing.

Nothing joins them, which costs two things.

**No single view.** `bd graph` renders the tree but knows nothing about agents.
`herdr agent list` knows the agents but nothing about the tree. To answer "what
is left, what is done, and what is being worked on right now" you read two
surfaces and hold the join in your head.

**Drift between them is invisible.** Observed live while writing this design:

| bead | bd says | herdr says |
|---|---|---|
| `nix-9670s.6` | closed | pane `wCM:p4` working |
| `nix-9670s.11` | closed | pane `wCM:pB` working |
| `nix-9670s.16` | in_progress | no pane carries this bead |

The first two are agents that finished and never exited. The third is a claim
with no visible agent — or an agent that has not identified itself yet, and
**today those two are indistinguishable**. That is the specific thing this
fixes.

## Goal

A tree of work rooted at a bead, with each node annotated by the live agent
working on it. The work is the spine; agents are an annotation that is absent on
most rows.

## Terminology

**Every term comes from beads or herdr. Where both are silent, and only then, we
coin one — and say so.**

| term | source | meaning here |
|---|---|---|
| bead, root, epic | beads | the unit of work; the bead a tree hangs from; `issue_type: epic` |
| tree | beads | a root and its descendants — everything that must finish before it can (see *Tree construction*) |
| parent-child, blocks | beads (`type` on a dependency) | the two edge kinds; a nesting is drawn from either, and the elbow says which |
| claim | beads (`bd update --claim`) | an agent taking a bead |
| stale | beads (`bd stale`) | in-progress with no recent activity, "may be abandoned" |
| ready | beads (`bd ready`) | open **and** every dependency satisfied |
| completed, progress | beads (`bd swarm status`) | the finished count and the `n/m` roll-up |
| active | beads (`bd swarm status`) | in-progress, an agent on it |
| `○ ◐ ● ✓ ❄` | beads (`bd list`'s legend) | open, in_progress, blocked, closed, deferred — glyph for glyph, because a glyph is terminology. `?` is `bdi`'s for a status bd has no legend for |
| agent, pane, session | herdr | the worker; its terminal; the server holding them |
| `display_agent`, `agent_status`, `state_labels` | herdr | read verbatim, never renamed |
| snapshot | herdr (`herdr api snapshot`) | one poll's whole state |
| **badge** | *coined* | a rendering of one metadata value. beads has `label`, but a label is a bead's own tag; this renders a `metadata` entry, which neither project has a display term for. |
| **unattributed** | *coined* | a live pane in a configured project resolving to no bead. Neither project names this, because neither knows about the other. |
| **unconfigured** | *coined* | a directory no `[[projects]]` entry covers, and the panes working in it. `bdi` has not failed to attribute them; it was never told the project exists. |
| **finished** | *coined* | a branch every bead of which is closed, with no agent and no anomaly anywhere beneath it — the whole branch, not merely its head. beads has *closed*, which is one bead's status; this is a claim about a subtree. |
| **run** | *coined* | several finished siblings drawn as one counted line — the code calls the line *elided* and the siblings a run. Neither project draws one. |
| **notice** | *coined* | something true of the view as a whole rather than of any row in it, said at the foot of the screen. |
| **freshness** | *coined* | how stale one project's rows are, said beside its name: a mark for how the read of it is going, and how long ago the rows were last read. Neither `bd` nor herdr has a word for it. |
| **armed** | *coined* | a project set to ask to be read again at a known instant. Neither project names it: the ask is `bdi`'s own. Armed by the read that came back and disarmed by the ask it makes, so a project always has a read outstanding or an ask armed — a project with neither is a project nothing will ever read again. A project with a producer and no poll is never armed. |
| **window** | *coined* | how long a read is held after it is asked for before it is sent, so that a burst about one project costs one read. It runs from the first notification and is not reset by the ones after it: under reset a held-down `^R` would withhold the read it exists to force. The screen says the read is coming when it is asked for, never when it goes. |
| **way down** | *coined* | the beads stepped through from a tree's root to a line. A bead reached more than once is drawn once per way down to it, and the way down is what tells the copies apart, what a fold and a selection are held by, and where a loop is cut. |
| **link** | *coined* | one way down from a bead to a bead beneath it, as the tree holds it: which bead, by which kind of edge, and whether it is the way the walk first reached the bead. beads has the dependency; the link is the nesting drawn from it. |
| **facts** | *coined* | what a line says of the tree beneath its bead — its fraction, what it is shut over, whether it rests open, whether it is finished, what a run under it stands for — and what a project's line counts over its trees. Each depends on the snapshot alone, so the forest answers them once when it takes a snapshot and a keystroke reads them. Neither project has a word for an answer kept between draws. |
| **ambient** | *coined* | the environment `bdi` itself was started in, which is what a project's tracker is read in unless the project's `environment` says otherwise. Neither project names it: `bd` reads whatever environment it is given, and herdr never runs `bd`. |
| **unanswered** | *coined* | a read of a project that has been outstanding longer than one may be and has produced nothing. Neither project names it: the read is `bdi`'s own, and neither `bd` nor `herdr` knows it is being waited on. Not *refused*, which is a read that came back and said no. Whether the read is the collection `bdi` is running or one queued behind it is not part of it — the reader's question is how long their rows have been on their way, and both answers to *why* are the same wait. |

### Three different things are called "blocked"

This is the trap the whole tool walks into, and one word for all three would make
it unreadable.

| name it | source | means |
|---|---|---|
| `status: blocked` | a bead's own field | somebody set that status |
| **not ready** | `bd ready`, `bd swarm status` | computed: an unmet dependency edge |
| `pane_status: blocked` | herdr `agent_status` | a TTY prompt is waiting |

They are close to disjoint in practice. A bead can be `status: open` and not
ready; a bead can be `status: blocked` with every dependency satisfied. `bd
blocked` reports the *edge* kind, not the status kind. The model keeps three
separate fields and the renderer never prints a bare "blocked".

## Scope

`bdi` assumes **bd and nothing else**. herdr is an optional provider that adds
liveness. It knows nothing about any particular way of organising agents — no
orchestration model, no workflow, no roles. It reads a tracker, reads a session,
and draws the join.

Anything workflow-specific is expressed as configuration, not code. See
*Conventions are configuration* below.

## Non-goals for v1

- **No writes to bd.** It surfaces drift; you act. A viewer that also repairs
  state is a second writer racing whatever else manages these beads.
- **No Noctalia widget.** The JSON contract is specified here; no widget ships
  until the TUI has proved the data model.
- **No cross-machine view.** One herdr session on one box. `herdr --remote` is a
  later consumer of the same collector.

## Architecture

Three units, each testable alone.

```
  bd CLI (per project) ──→  collector  ─→  model  ─→  ┬─→  TUI (ratatui)
  herdr socket (optional) ─┘             (pure)      └─→  --json (serde)
```

**collector** — the only unit that does I/O. Shells out, parses, joins.
**model** — pure functions: tree construction, anomaly rules, ordering,
filtering. All the logic worth a test lives here and needs neither herdr nor bd
to run.
**renderers** — two consumers of the same model.

## herdr is a provider, not a dependency

**bd discovers the trees. herdr filters and enriches them.**

An earlier draft had herdr enumerate the roots. That is wrong, and the evidence
is concrete: the tracker held a root with no pane in herdr at all. Herdr-first
enumeration drops it silently.

So `bdi` runs in two tiers:

| | bd only | + herdr |
|---|---|---|
| tree of work, correctly drawn | ✅ | ✅ |
| done / left / in-flight counts | ✅ | ✅ |
| who claimed it, and when | ✅ | ✅ |
| configured metadata badges | ✅ | ✅ |
| stale claim | by age — a heuristic | exact |
| agent alive right now | ✗ | ✅ |
| `stale-pane` and `unattributed` | ✗ | ✅ |
| pane tail and focus | ✗ | ✅ |

The age heuristic earns its keep alone. In one tracker, two beads have sat
`in_progress` for 54 and 58 days. No herdr needed to see that. It stays a
warning rather than a verdict, because a genuinely long-running bead trips it.

### The default filter

**When herdr is available, the default view is trees with at least one live
agent** — the work actually in flight. Trees with no agent are not dropped; they
collapse to a single line so nothing disappears silently:

```
▸ 4 trees with no live agent             a to show all
```

`a` toggles to the unfiltered set, and the choice is the reader's: it survives
a refresh, as the folds and the cursor do. When herdr is unavailable there is
no filter to apply, so every discovered tree renders, and a notice at the foot
of the screen says that which agents are alive is unknown.

## Discovery

Roots come from bd, unioned and deduped:

1. Every unfinished bead — `open`, `in_progress`, `blocked` or `deferred` —
   and every unfinished wisp, walked to its root via `parent` ancestors. This
   uses bd's own statuses and needs no convention, and it is read off the
   `list --all` and `query ephemeral=true --all` answers the forest is drawn
   from rather than asked of bd as a subset of them: measured 2026-09-02
   against this project's tracker, every row `--status` listed was in the
   `--all` answer, and the same for wisps. An earlier draft took only
   `in_progress` and `blocked`, which turned a tracker into a handful of
   roots; since `dd2c3b5` the climb starts from every unfinished bead, so
   every tree with anything left to do is drawn.
2. Roots named explicitly in config, or as `bdi <bead-id>` arguments. Both
   carry the project whose tracker holds the bead, because the key is
   `(project, id)`: config lists the ids under the project, and an argument is
   written `<project>:<bead-id>` — bare where there is only one project, which
   is the whole of a zero-config run.
4. Any bead named by a live pane's `display_agent` that the first three missed.
   This is the only root herdr contributes, and it exists so an agent working
   off-tree still appears.
5. Any bead the answer holds that the first four leave no way down to: a
   dependency that would have nested it names work the tracker no longer
   holds, and no surviving edge nests it under anything. Such a bead is the
   top of its own graph. A tree reports the beads it drew, so a bead no tree
   draws is a bead no tree reports — drawing it is what leaves it somewhere to
   be reported from.

## Scoping a run to fewer projects

A run reads some of the configured projects and never the rest. It is not a
view filter: the projects left out are never read. Which projects those are
is a function of the config, the directory `bdi` was started in, and the
command line, decided as the config is assembled and before git is asked
where each project is worked — which is itself a subprocess in the project's
own directory. The config is not narrowed. It keeps every project it names,
and carries beside them the *scope*: which of them this run reads, and what
chose them. Every site that gathers reads the projects through that scope —
the collection loop, the working trees git is asked for, the order the trees
are drawn in, the forest drawn before any tracker has answered, the names the
inbound channel answers `ok` to — so one decision reaches all of them and
none of them needs to know about it. The sites that place a pane read the
projects as written, which is what lets a pane on another desktop be placed
in its own project rather than reported as in a directory nobody configured.
See *The excluded projects stay known* below.

**The directory `bdi` is started in decides the read set.** The use is one
`bdi` per desktop: a desktop's panes sit under one project's directory, and a
`bdi` started there should draw that project's trees and nothing else. The
project that holds the current directory is the one read — *holds* being the
test the join already uses to place a pane: under the project's path or any
of its working trees, deepest match winning, so a repository inside another
resolves to the inner one, and a launch from a repository inside a project's
directory lands on that project. Started outside every configured project,
`bdi` reads everything: there is nothing to scope to and nothing was asked
for.

One case that test misses is a linked worktree placed outside the project's
tree, because the scope is decided before any project has been asked where it
is worked. It is covered with one git call *from the current directory* for
the working trees of whatever repository it sits in, and the directory's
counterpart in each of them is tried against the same test — each, because a
config may name a project by its place in a linked worktree rather than the
main one. Nothing runs in any other project's directory.

**`--all-projects` opts out and reads every configured project.** It is what
the session watching everything from one project's checkout runs. It cannot
be `--all`: that flag draws every tree, including those with no live agent,
and keeps that meaning. `--all-projects` with `--project` is a command line
asking for every project and for only some, and is refused as the
contradiction it is, the way `--poll --no-poll` is.

**An explicit `--project <NAME>`, repeated for more than one, outranks the
directory.** It stays the way to ask for two projects, or a different one,
from anywhere, and the directory is not consulted. `--project` has no path
form; the directory is the path form.

**The no-config run is unchanged.** It discovers the project the current
directory sits in and reads that; the rule above is that behaviour with a
config present. The one project it discovers is everything there is, so the
run reads it as everything and the screen has nothing to say about a scope.

Never reading the rest matters because a collection is most of what a run
costs, and the cost follows the number of projects rather than the size of
any one tracker. A
project whose tracker has moved is read in full: four `bd` invocations
whatever the tracker holds — `ready`, `blocked`, `list --all`, `query
ephemeral=true --all` — all of them after the environment its tracker is
read in is settled, which is a process of its own only where the project
named direnv or a credential command. Counted off `collect::bd`
once discovery read the listing rather than three subsets of it (`bdi-9jj.8`);
before that it was six, plus one `show` per closed parent the climb stepped
onto. A project whose
tracker has not moved pays one `bd sql` probe and none of the rest — see
*Reading a tracker only when it has changed*. Measured on 2026-09-01 at
`268ab2d`, before that gate landed: `bdi --json`, which waits for the whole
collection and draws no screen, took 8.2 seconds against a config naming
three projects, and 3.5 to 4.1 seconds against two at `40f4eb5`.

`bdi` itself no longer makes the reader wait that out — since `cfcbd80` the
forest is on the screen in about 22 ms and fills in as trackers answer — so
what scoping buys the reader is not the first frame but every collection after
it, and the whole of `--json`. Nothing bounds how many projects a config
names.
**A scope naming no configured project is refused, and the refusal lists what
is configured.** The likely cause is a typo, and the alternative is starting on
an empty forest the reader cannot tell from a quiet one. This is what the file
already does with an unknown project name in `[roots.explicit]` or in a
`<project>:<bead-id>` argument.

What decides whether a run is scoped is whether a project holds the directory
or a `--project` was typed, never how many projects a `--project` selected. A
scope that selected nothing is refused rather than obeyed, and a run scoped by
neither reads everything — the two must not be reached through the same
emptiness test.

**Scoping by `--project` is silent, and this is a deviation from *degrade,
never disappear*.** That principle governs a tree `bdi` could not draw: an
unreachable tracker, a dangling parent. A project the reader excluded on the
command line is not a failure to report, and a standing line about it would
be noise on every run of a flag whose whole purpose is a smaller screen. The
reader typed the scope; the screen does not need to tell them what they typed.

**A scope the directory chose is said on the screen.** The reader did not
type this one, which is weaker ground for silence, and a reader who sees one
project could think the others vanished. One line below the groups, in the
shape the hidden-trees group takes — no warning mark, and the way to the rest
where that group keeps its key — names the project being read and says that
`--all-projects` reads every project. That is the degrade-never-disappear
answer: the projects left out are not drawn, and the screen says so.

**The positional `<project>:<bead-id>` still adds a root, and does not scope.**
The two arguments do different jobs: `--project` decides which trackers are
read, the positional adds a root inside a tracker being read. Merging them
would remove the ability to add a root while still reading everything, which
is what the positional does from outside every configured project.

Scoping is applied before the roots the command line names, and what a
positional under a project the scope left out means depends on which kind of
scope it is. Against a scope the reader typed it is refused. The
contradiction is inside one invocation — the same command line asking for
`beta`'s root and asking not to read `beta` — and there is no reading of it
under which both halves are meant. The other order accepts it and then draws
nothing: `roots.explicit` is read only inside a project's own collection, so
a root under a project no collection reaches is dropped with nothing said
about it. Refusing is the degrade-never-disappear answer here rather than the
price of it. Against a scope the directory chose there is no contradiction,
because the reader asked for nothing the root contradicts: the root widens
the read set to take its project in, so `bdi homelab:hl-123` from the Beacon
desktop reads Beacon and homelab. The widening happens before git is asked
where each project is worked, so the project a root brought in learns its
working trees like any other.

A root the **config file** names under an excluded project is not that
contradiction, and is silent. It is a standing preference the reader is
overriding for one run, so its tree is one the reader excluded rather than one
`bdi` could not draw — which is the rule that makes scoping silent in the
first place, applied to a root instead of a project. The entry stays in
`roots.explicit` rather than being pruned, because nothing consults it for a
project no collection reaches.

One consequence worth knowing: a scope that leaves exactly one project makes a
bare bead id unambiguous, because what a bare id was ever ambiguous about is
which of the trackers being read holds it. `bdi --project orbital orb-7` works
against a config naming three, and so does `bdi orb-7` from orbital's
checkout.

**Considered and rejected: widening an explicit `--project` by the projects
the positionals name.** It would let `bdi --project alpha beta:xyz` work by
putting `beta` in the scope because a bead of `beta` was named. What sinks it
is that it infers an opt-in the reader cannot see: `--project alpha` would
read `beta`, and every project line on the screen looks like one they asked
for, so there is nowhere to notice it. Refusing costs one word to recover
from; reading an excluded tracker is not observable at all. A scope the
directory chose is widened, and the difference is that the reader typed
nothing the widening overrides — and the screen says the directory chose, so
a project line beyond the one named there is visibly one the reader added.

**The excluded projects stay known to the run.** The config as written stays
reachable alongside the read set, rather than the projects being narrowed to
the read set and the rest dropped. Two things need it now. The join places a
pane by which configured project holds its directory, so a scoped run whose
projects were only the read set would report every pane on the other
desktops as *in a directory no configured project covers*, and every
unreadable tracker as *possibly more*. Panes are placed against the config
as written, and a pane under an excluded project is neither drawn nor
reported: not loose, because it is on another desktop's work; not
unconfigured, because the config names its project; and not a claim on a
read project's bead of an id it names, because its own tracker was never
read and says nothing about that bead. A read bead naming a pane that sits
in an excluded project is still reported, as a pane in that project. And
reloading the config while running will have to re-derive the read set from
the new file, so it is kept a function of config, directory and flags rather
than a value computed once at start. Later, a reference that crosses
trackers will need it to resolve and show a foreign bead from a scoped run.

## Conventions are configuration

Different setups encode different things in bead metadata. `bdi` hard-codes none
of them. Its config names which keys to notice:

```toml
[roots.explicit]                        # roots named outright, per tracker
orbital = ["orb-7"]

[[badges]]                              # rendered as a marker on the row
key    = "delivery_pr"
render = "⇢ {}"

[[badges]]
key    = "blocked_on"
match  = "human"
render = "⏸ waiting"
```

Nothing in the model knows what `delivery_pr` means. It knows a key was
configured, found on a bead, and should be drawn. A setup with different keys —
or none — works the same way.

## Data sources

Verified against a live session, 2026-08-30.

### bd

Every call `bdi` makes to bd is spelled in one place, `collect::bd::asked`, and
the roster is short: `bd list`, `bd query`, `bd ready`, `bd blocked` and one
`bd sql` probe. `bd dep tree` is not among them. An earlier draft of
this section described a per-root `bd dep tree <root> --direction=up --json`
walk as the tree source; that call was replaced and its row shape survives
nowhere in `bdi` (`bdi-r95`, `bdi-7ao.12`).

**The tree source is one `bd list --all --limit 0 --json` per project.** It
carries every bead the tracker holds, and each row names every bead it depends
on in a `dependencies` array of `depends_on_id` and `type` — the whole graph,
complete, in one call. Timed three times each against this project's own
tracker on 2026-08-30 with bd 1.2.2: 133–204 ms, against 1454–2292 ms for `bd
dep tree` over the largest root. One call per *project* rather than one per
root, and it is what makes drawing a blocker under every bead it blocks
possible at all: `bd dep tree` is a spanning tree, not an edge set — bd dedups,
so each bead comes back carrying only the one edge the walk first reached it
by (93 of 176 edges for this tracker's largest epic, same date, same bd). Both
flags are load-bearing. `--limit 0` lifts a default of 50 that truncates
visibly; `--all` lifts a default of open-only that returns a smaller,
correct-looking answer about a different population.

A row carries:

| field | use |
|---|---|
| `id`, `title`, `status`, `priority`, `issue_type` | the row |
| `parent` | the bead's own parent, as bd holds it — not a traversal's |
| `dependencies[]` — `depends_on_id`, `type` | every edge out of the bead; `type` is `parent-child` or `blocks`, and any other value nests nothing |
| `metadata` | the whole map, inline |
| `updated_at`, `started_at`, `closed_at`, `owner`, `assignee` | the age rules |

Three consequences:

- **`bdi` builds the tree.** `model::tree::assemble` walks the edges from each
  root and puts every bead where its edges say it goes, one copy per way down
  to it. A blocker is drawn beneath the bead it blocks because that walk puts
  it there. See *Tree construction* for the rule.
- **A bead is drawn once per path, and that is the model rather than an
  accident.** `bd dep tree` dedups; `bd list` is an edge set, and the tree
  built from it has as many copies of a bead as there are ways down to it.
- **Badges need no second call.** `metadata` is inline per row, so a configured
  key is read from the row that already loaded.

**`bd query ephemeral=true --all --limit 0 --json` supplies the wisps**, the
ephemeral beads `bd list` never lists. A wisp with no parent would otherwise be
collected and hung nowhere.

**`bd ready --limit 0 --json` supplies readiness**, and `bd blocked --json` the
blocker set. Neither is computable from the rows: a row names what it depends
on, not whether those dependencies are satisfied, and beads already answers
that. `bd swarm status` shows Ready as a first-class state alongside Completed,
Active and Blocked, so a viewer that collapsed Ready into plain "open" would be
throwing away a distinction beads makes. One call each per project, intersected
with the tree's ids.

**`bd list --all --limit 0 --json` and `bd query ephemeral=true --all --limit
0 --json` supply discovery**: every unfinished row of either is a root
candidate. **The climb to a root is answered from the rows already read**:
every `bd list` row carries the bead's own `parent`, so a closed bead above
open work — the shape discovery never names — costs no further call.

**`bd sql --json "SELECT dolt_hashof_db() AS h"` is the probe** that gates all
of the above — see *Reading a tracker only when it has changed*.

What we still redo is the **rendering**. bd's text tree emits broken glyphs —
vertical connectors missing under a node that has following siblings, and child
indent that does not line up with its parent's marker. The JSON is sound; only
the drawing is not.

### herdr

`herdr agent list` returns JSON over the session socket. The fields that matter:

| field | use |
|---|---|
| `pane_id` | the join key, e.g. `wCM:p9` |
| `cwd` | resolves the pane to a project root |
| `display_agent` | the bead id an agent stamped |
| `title` | the agent's one-line "what I am doing" |
| `state_labels` | per-state text, shown for the state the pane is in |
| `agent_status` | `idle` / `working` / `blocked` / `done` |

`herdr agent read <pane>` gives terminal output for the tail pane.
`herdr agent focus <pane>` is the only write `bdi` performs, and it writes to
herdr, not to any system of record.

**`agent_status: blocked` means a TTY prompt is waiting** — a permission gate, or
a pane still at a startup confirmation. It is a property of the terminal, not of
the work, and it must never be conflated with a bead's status or with any
configured badge. The model keeps them in separate fields.

There is no event stream. `herdr agent list` prints live state; the collector
polls it once per collection.

## The join

Bidirectional, because each direction alone has a hole.

**pane → bead** is herdr's `display_agent`, when an agent has set it to a bead
id. Its hole: an agent that has not identified itself carries no bead, so its
work looks unstaffed.

**bead → pane** is a metadata key naming the pane:

```bash
bd update <id> --set-metadata agent_pane=$HERDR_PANE_ID
```

Its hole: a setup that does not write it.

Together they close both. A bead is **live** if either direction resolves to a
pane present in `herdr agent list`. The pane id exists from the moment the pane
does, so there is no race against an agent that has not identified itself yet —
which is what makes drift detection exact rather than a guess.

`agent_pane` is the default key name and is configurable. `bdi` never requires
it: without it, liveness falls back to `display_agent` alone and the affected
rows say so.

### The join is scoped to one project, and conflicts are reported

Two rules that a naive implementation gets wrong.

**A pane joins only to its own project's beads.** A pane's `cwd` resolves it to
a project by the longest matching working tree of that project. A project's
working trees are the place it names, in every working tree `git worktree list`
reports for its repository — so a project configured as a directory inside the
repository is that directory in each of them, and never the repository around
it. A pane belonging to no configured project joins nothing and lands in
`unconfigured`; a pane in a configured project that no bead there claims lands
in `unattributed`. The two are kept apart because only one is the reader's to
fix, and the fix is a config entry rather than a bead.
One worktree per seat is a common way to work, and it puts the panes under
neither each other nor the checkout `bdi` was run from, so a project that held
only one directory staffed nothing. Without this, two trackers with colliding
id prefixes cross-attach agents — and prefixes are per-tracker and
uncoordinated, so a collision is a matter of time rather than bad luck.

**Where the two directions disagree, that is a finding, not a tie to break.**

| situation | what `bdi` does |
|---|---|
| `agent_pane` and `display_agent` name different panes | the bead's own key wins; the disagreement is reported |
| several panes name one bead | none wins; reported |
| one pane is named by several beads | none wins; reported, with what the pane says it is working on |
| a pane's project differs from the bead's | no join; reported |
| a pane's directory is under no configured project | no join; reported once as `unconfigured`, and on each bead whose key named it |

Silently picking one is the failure mode: each of these is drift of exactly the
kind the tool exists to surface, and last-write-wins would hide it behind a
plausible-looking row.

**A contested pane says what it is working on, in its own words, and `bdi`
still picks no winner.** The conflict carries the pane's caption — its state
label for the state it is in, falling back to its title — and the row reporting
it quotes that caption directly after the pane id and ahead of the roll of
claims, because a sentence too long for the width is cut from the right. It is
the one thing on the screen that can tell the live claim from the stale ones: a
caption is otherwise drawn off the agent a pane was awarded, and a contested
pane is awarded to nobody. `bdi` reads nothing out of it — no bead id is parsed
from it, nothing is matched against the claims — so the reader decides, which
is what keeps this a finding rather than a tie broken.

What `bdi` will **not** do is rank the claims by `started_at`. The newest claim
reads as the live one only under one assumption about how agents are organised
— a single seat moving between beads and forgetting to clear its key — and it
is wrong for two seats where one died, since nothing says the survivor started
later. `started_at` stays parsed and unused.

**A refused claim carries its refusal.** Where a bead's own key named a live
pane the join would not award it — because the pane sits in another project,
or in none, or because several beads name it — the bead's `orphan-claim`
anomaly carries that conflict as its reason, and the row says it: *claimed ·
its pane is in no configured project*, *claimed · 3 beads name its pane*. A
bead whose row said only *claimed · no pane* sent the reader after a dead agent
that was alive and working two feet away. Only the exact direction can be
refused: a bead that named nothing has no claim to refuse, and a pane naming an
id another tracker happens to reuse says nothing about that tracker's bead.

## Anomaly rules

All computed in the pure model. The first needs bd alone; the rest need herdr.

| rule | condition | reading |
|---|---|---|
| `stale-claim` | `in_progress`, not updated in N days | beads' own `bd stale`, narrowed to claims |
| `orphan-claim` | `in_progress`, no pane resolves for it | the agent died mid-claim — or the bead named a live pane the join refused it, in which case the refusal travels with the rule and the row says which |
| `stale-pane` | bead is closed, its pane is alive | agent finished and did not exit |
| `unattributed` | pane alive in a configured project, no bead resolves | a pane nobody can account for |
| `unconfigured` | pane alive in a directory no `[[projects]]` entry covers | a project `bdi` was never told about; the fix is a config entry |

`stale-claim` is `bd stale` restricted to `in_progress`. Its window defaults to
**30 days, matching `bd stale --days`** — not a number of our own. Two names stay
apart deliberately: `stale-claim` is about a bead nobody has touched;
`stale-pane` is about a pane that outlived its bead. They share a word because
both are "this outlived its usefulness", and nothing else.

**A node carries every anomaly that fires, not the first one.** An old claim
whose agent has died is both `stale-claim` and `orphan-claim`, and reporting only
the second throws away how long it has been sitting there — which is the part
that tells you whether to care. The field is a list, and it is `anomalies: []`
on a node nothing fired on — never absent, never null.

`orphan-claim` keys on `in_progress` alone. A bead that is `status: blocked` with
a live pane is not an anomaly — an agent parked on it is a normal state, and
firing on it would report every waiting agent as dead.

`orphan-claim` has one shape in the JSON whatever its reason: `{"rule":
"orphan-claim"}` for a claim that really did lose its agent, and the same with
a `refused` field holding the conflict where the join refused the bead's own
key. The field is omitted rather than null when there is none, so the two are
byte-identical up to it. `Conflict` was already contract surface as the
top-level `conflicts` array, so nesting one here exposes nothing new.

`unattributed` will also catch ordinary interactive sessions, which are not
anomalies. It renders as a group of its own below the trees, never as an error
against a tree, and rests open, because what it holds is live. `unconfigured`
renders as a group of its own above it, naming each directory, because the
sentence a reader needs there is about the configuration rather than the pane.

## Tree construction

1. Discovery yields the roots.
2. Per project, one `bd list --all --limit 0 --json` plus the wisps, and the
   tree under each root is built from the edges those rows carry.
3. `herdr agent list`, if reachable, is joined onto the nodes.
4. The default filter collapses trees with no live agent to a count.

**What a nesting means.** A bead's descendants are the things that must
complete before it can. beads says that with two edge kinds running opposite
ways, and one rule lands them both: a parent cannot finish until its children
do, so a child is drawn under its parent; a bead cannot finish until its
blockers do, so a blocker is drawn under the bead it blocks. An edge kind beads
may add later has no settled direction against completion, so it nests nothing
— the bead still appears wherever its other edges put it. A nesting therefore
never means "the walk reached this through that", and every row carries which
kind of edge put it where it is. Rejected on the way: drawing each bead's
dependencies as its children uniformly, which fixes `blocks` and inverts
decomposition, so an anchor epic with no dependencies becomes a leaf repeated
under every one of its own descendants; and not nesting `blocks` at all, which
fixes the count and loses the thing the count is for — seeing, in one place,
everything standing between a bead and done.

**The elbow says which.** A child hangs on a solid arm, `├── `; a blocker hangs
on a dashed one, `├┄┄ `, and shut `├┄▸ `. So a bead drawn under its parent and
again under something it blocks is two true statements with two different arms,
not one row twice. The arm belongs to the edge and not to the bead: a child of
a blocker is drawn on a solid arm under the dashed one. A run of closed
siblings is a count, not a bead, and keeps the solid arm whatever it holds. The
prefix stays four columns a level, and the fold marker stays inside the elbow.
No colour carries it, because colour is never the only channel here and the
box-drawing stays in the terminal's own foreground so a reader can follow the
rules down. Sibling order is unchanged — status, priority, id — so blockers and
children interleave, and the arm is the one thing that says which each is.
Terminology is beads' own: `parent-child` and `blocks` are the two `type`
values bd writes on a dependency; nothing is coined.

**Why a bead is drawn more than once.** A bead has one parent and any number
of beads it blocks, so it has as many places as there are ways down to it, and
it is drawn at each. That is the normal case rather than an exotic one: 50 of
100 beads drawn more than once, measured against this project's own tracker on
2026-08-30 after a dependency audit (506 rows over those 100 beads, same
measurement; the totals move whenever an edge does, and this one moved twice
in an hour). A copy is identified by the way down to it — the tree, and the
beads stepped through below its root to reach the line — not by the bead, and
that is what a fold and a selection are held by. Folding one copy leaves the
others as they were.

**The tree holds each bead once, and the ways down to it point at it.** The
copies are drawn, not stored. A tree is every bead its root reaches, held once
— the root first, then the rest in the order a walk down from it first reaches
them — and, for each, its links: the ways down from it to the beads beneath
it, each saying which bead, by which kind of edge, and whether it is the way
the walk first reached the bead. What the screen draws is that tree unrolled,
a line for a bead at every way down to it whose forebears are open, and the
layout walks the links carrying the way down as it goes. Every question it
asks of a line — what is beneath it, how far along it is, whether it rests
open, what a run stands for, whether it is the first line of its bead — is
answered from the bead and the way down to it: reachability from the bead
with the way down left out, each bead once, or for the first line, whether
every link down the way is a first link. So a question costs the size of the
tree and not of the unrolled shape, which can be very much larger: measured
2026-09-02 against the maintainer's five trackers, 336,063 unrolled rows over
4,611 beads, one tree of 119 beads unrolling to 194,085 of them, and a
keystroke under `--all` that cost 146 ms over the rows and 5 ms over the tree.
The unrolled shape is walked whole in one place, `--json`, which writes it.

**The facts are answered once per snapshot.** Every one of those questions
depends on the snapshot alone — no fold and no selection moves an answer — so
the forest answers them when it takes a snapshot and layout reads them, rather
than asking again for every drawn line on every keystroke. A tree with no loop
in it keeps one answer per bead: nothing beneath a bead can be above it, so
leaving the way down out changes nothing and every copy of the bead reads the
same answer. A tree with a loop cut in it is still asked by the way down,
because two copies of a bead on either side of the cut stand over different
things. Measured 2026-09-02 against the maintainer's five trackers, 6,060
beads held across the shown trees under `--all` and 3,560 lines drawn: the
keystroke went from 5.0 ms, 3.1 ms of it those questions, to 2.9 ms, 0.8 ms of
it, and answering them once costs 7.7 ms per snapshot.

**Dedup is the model's; the copies are the view's.** The model holds one node
per bead and the view draws one line per way down to it, and the two are not
in tension. An earlier draft warned that one node per id makes id-based
navigation land on whichever copy was built last; that held only while the
fold and the selection were keyed on the id. Both are keyed on the way down to
a copy, so they land on the copy the reader is standing on, and there is no
last-built copy to lose to. What must not be duplicated is the *identity*:
`(project, id)` names one bead however many lines carry it, and asking what a
bead *is* — which agent is on it, say — resolves the id to one answer, because
the agent belongs to the bead and not to the copy.

**What a fraction counts.** A line that stands for more than itself says how
much of that is done: a node with children gets closed/total over its whole
subtree with its own bead among the total, and a root gets the same, which
stops the root being a special case at all. A leaf gets a glyph and no count,
because a fraction over one bead only repeats its glyph. Counted as beads, not
rows — every distinct bead beneath the line once, however many ways down reach
it — so it answers "how much of what I am waiting on is done", and a closed
bead cannot report a fraction over beads that merely waited on it, because
those are not beneath it any more. A project's line counts over its trees with
each bead counted once, for the same reason. Keyed on having children, never
on `issue_type == "epic"`: reading a display rule out of bd's taxonomy is
interpreting what a field means, which this project's rules push into config,
whereas having children is the shape of the tree `bdi` already computes and is
exactly the condition under which the question is askable.

**Two degradations, and a cycle.** `dangling` is beads naming something they
depend on that the answer does not hold — most often a deleted parent; each is
still drawn, and a bead nothing in the answer nests at all is a root of its own
(discovery rule 5), so it is drawn under its project and reported as dangling
there rather than nowhere. `cycles` is beads whose own descendants lead back to
them — a bead blocked by one of its own forebears, which beads permits — each
still drawn, where the loop was cut. The cut is the way down: a walk that
comes back to a bead it came down through stops there, so the tree holds the
way back up as a link like any other and every walk declines to take it.
Which beads are reported is therefore a fact about the walk and not only
about the loop — a bead met above a loop is cut when the loop comes back to
it, and one met only from inside the loop never is — and two copies of a bead
on either side of a cut are the one place two copies stand over different
things. An earlier draft called the second
`unreachable` and hung such beads off the root; under this rule every bead is
drawn where the rest of its edges put it and it is the loop that is cut, so the
old word would have been false on a contract field. That is a JSON contract
rename (`trees[].unreachable` → `trees[].cycles`), free when it was made
because nothing consumed the contract.

Ordering within a level: state first (in-flight, then blocked, then open, then
closed), priority second, id third.

### Folds, elision and what rests open

**The invariant, stated once: no live agent and no anomaly is hidden by a fold
`bdi` chose — not by a fold, not by an elision, not by a filter.** The
live-agent filter already reasons this way in the other direction, collapsing a
tree with **no** live agent into `hidden_trees`; this is its dual. Three
mechanisms each used to break it — an elided run's phrase denying an agent its
count included, quiet tested on a sibling and applied to its whole subtree, and
a default fold that hid agents nobody had navigated to — and each is closed
below. A reader may fold over live work by hand; that is the one place the
rule yields, and it yields because they asked by name.

**The default: open the spine to the work a reader needs on the first screen,
and nothing else.** A line rests open exactly when something beneath it is a
live agent, an anomaly, or a bead `bd` calls ready. Nothing else opens a fold:
unfinished work that is blocked or deferred does not, because readiness is
`bd`'s own answer and not a status test. The bead that earns the fold does not
open its own; only its forebears open, so the screen is that work and the spine
down to it. Not full expansion, which is a wall of closed work; not
collapsed-except-selected, which was never built — the selection has no bearing
on what is open. With the filter on, every tree drawn has an agent, so every
tree opens; with it dropped, a quiet tree is a shut line. The two alternatives
were drawn against the real tracker before Graeme chose: annotating a shut line
alone still costs a keypress to see ready work, and opening to every unfinished
bead put roughly 34 of 77 nodes on screen plus their spines, which is close to
having no fold at all. Measured at `3385907` against `279e71b` on this
tracker's 81 nodes: 35 lines at rest before, 39 after.

**A fold set by hand stands over what it folded away, and is spent when a
refresh brings something live beneath it that was not there before.** So a
hand-fold survives any number of refreshes while the work under it is unchanged
or dying down, and an agent arriving on a bead the reader never saw hands the
node back to the default. A fold means "I have seen this and do not want it",
which stops being true the moment something new is under it. Only hand-folds
are stored; the default is derived, which is what makes "back to the default"
a single key.

**A finished branch draws as one line and rests shut.** Its glyph, its
fraction and its fold marker already say *finished, and holds more*; opening it
is the ordinary fold. A closed bead standing over unfinished work is the
ordinary shape of a tree walked by dependents — closing a blocker is how work
proceeds — and it is not finished: it collapses to one line and says how much
it holds, *3 unfinished beads beneath this*, unless some of that work is ready,
in which case it rests open. A closed branch is drawn open, in spite of all
this, when a live agent or an anomaly sits anywhere inside it. A shut line over
agents or anomalies says so beside its fraction — *◍ 2 agents beneath*, *⚠ 1
bead beneath* — counted as beads, and strictly beneath, because the line's own
agent is already on the row by name.

**A run of finished siblings is drawn as a count where there are enough of
them.** Three or more finished siblings collapse to one line, `✓ 13 more beads
· closed, and nobody on them`, carrying `✓` because closed is the one state
every member holds, and counting them and their whole subtrees. Because its
members are finished branches, its phrase is true of every bead it counts, not
merely of the siblings it names. Fewer than three are drawn: `… 2 more` costs a
line and saves one. The threshold was chosen against this tracker's shape
rather than Graeme's word (his word was *many*): seven nodes had one finished
sibling and three had six or more, so it decided exactly one node. A closed
sibling carrying an agent or an anomaly is never in a run — that is the
stale-pane case, and eliding it would hide a live agent.

**A run is a fold like any other.** It is selectable, opens with the keys that
open a bead's children and shuts with the ones that shut them, and its fold
survives a refresh. Opened, its beads hang *under* the run line rather than
splicing back into the parent's sequence: a run is always the last of its
parent's entries, so a sibling drawn after it at that depth would follow an
elbow that had already said it was the last, and nesting keeps the box-drawing
well-formed and lets `h` and `l` step out of and into the run through the depth
arithmetic that was already there. An open run re-elides inside itself: the
three-or-more rule is a property of the forest and not of a place in it, so a
sibling whose own children are a finished run draws a nested count, and the
account still reconciles because the count is recursive. Drawing flat instead
would mean opening `… 143 more` dumps 143 lines that can only be folded back by
shutting the whole run. The marker convention holds here as everywhere: a shut
run says so in its elbow, `└─▸ ✓ 2 more beads …`, and an open one says it by
drawing its beads beneath.

**The cursor rests on anything with an identity that survives a refresh:** a
bead's row, a root that would not read, a project line, a run, a group line and
each thing inside a group. A note — a per-tree finding drawn under a project's
roots — stands for a finding rather than a thing, has no such identity, and the
cursor steps over it. That is the same question as whether the selection can
be put back after a refresh, which is why the two have one answer.

## Cross-project credentials — the hard constraint

Where several trackers live on one Dolt server, `bd` uses **one database and one
user per project**, with the password supplied per project through the
environment. So a process holding one project's credential cannot read another's:

```console
$ bd -C ../other-project list
Error: failed to open database: ... Error 1045 (28000): Access denied for user 'other-project'
```

bd found the config — it knew the database and the user — and had no password
for it. **This is a credential boundary, not a policy one**, and it is the
biggest constraint on the multi-project view.

### How a project's tracker is reached is a choice per project

Confirmed with the operator of this deployment: no cross-project reader exists
today; the one read-only user on the server is scoped to a single database.

A project is read in one of three environments, and its config entry names at
most one:

| `[[projects]]` says | the tracker is read in |
|---|---|
| nothing | the ambient environment, with the credential the launching shell holds |
| `environment = "direnv"` | what entering the project's directory produces |
| `credential_command = "…"` | the ambient environment, with the command's stdout as the credential |

**Ambient is the default**, because it is the run a new user makes first: a
machine with bd and nothing else reads the tracker its shell can already
reach, and direnv is a dependency `bdi` does not otherwise have. `-C` naming
the tracker outright is what makes that safe, and the rest of this section
says why. direnv was the default before `-C`, when entering the directory was
the only safe way to reach the right tracker; it is now how a setup with one
credential per project supplies them, and that is a setup rather than the
tool, so it is asked for by name. Inferring it from an `.envrc` and a direnv
on PATH was declined: explicit costs one line, and the config then says which
mechanism reads a project where an inference would have to be re-derived to
be reported.

**Neither a credential nor a tracker path is carried by a working directory.**
An earlier draft said `bd` finds a project's credential by being run in that
project's directory. It does not. `BEADS_DOLT_PASSWORD` reaches an interactive
shell through direnv, and a child process inherits **the parent's** environment
whatever its working directory is. So a single process that merely changes
directory authenticates every tracker with whichever credential it started
with — silently, and against the wrong database only when two trackers share a
name.

**The same sentence is true of the tracker path, and the first fix missed it.**
`BEADS_DIR` names the tracker and outranks the working directory, so a `bdi`
launched from a shell scoped to one project read that project's tracker for
every project it was configured with. Which tracker is read and what
authenticates to it are one identity: a child is told both or neither.

**A shell that has entered a project's directory is correctly configured for
its tracker.** direnv is what makes that true — it loads the flake, the bd
version, `BEADS_DIR`, and whatever holds the password. So a project that asks
for direnv is read by reproducing entering the directory rather than by
reconstructing what entering it would have produced, and the entry says
nothing about what the secret is called or where it lives:

```toml
[[projects]]
name = "summit-works"
path = "/tmp/bdi-ground/summit-works"
environment = "direnv"
```

- **The tracker is named outright, with bd's own `-C`.** Every call `bdi` makes
  is `bd -C <project.path> --readonly …`. `-C` outranks `BEADS_DIR` in both
  directions, measured: `BEADS_DIR=/nonsense bd -C <project>` resolves the
  project, and `BEADS_DIR=<valid> bd -C /tmp` refuses with *no beads project
  found*. Stating the tracker does not depend on `bdi` having thought of every
  variable bd reads.
- **`-C` is what makes the ambient environment safe, and entering the
  directory with it.** direnv fails open: it exits 0 and runs with the ambient
  environment where an `.envrc` is unallowed or a flake will not evaluate.
  With the tracker named outright, neither the ambient environment nor such a
  fallback can point bd at the wrong database — only fail to authenticate
  against the right one, which `bdi` reports per project as `auth` while every
  other tree still draws.
- **`--readonly` has bd enforce the no-writes rule** rather than leaving it to
  `bdi` being well behaved.
- **The environment is captured once per project, not per call.** `direnv exec`
  reloads the directory every time it runs. Measured 2026-08-31 with direnv
  2.37.1 (the version this machine ran then and still does at the time of
  writing; it is direnv's behaviour and this repository's `.envrc` that decide
  this figure, not any `bdi` commit): 20ms of overhead on `summit-works` but
  **1.3 to 2.4 seconds** on this repository, whose `.envrc` watches the
  profile file its own nested `.envrc.local` rewrites, so the cache is
  invalidated by the previous load every time. A whole collection of both
  trackers cost 2.5 to 2.7 seconds that day, and `bdi` makes seven or more bd
  calls per project when a tracker has moved, so per-call was never
  affordable.
- **A project whose `.envrc` writes to stdout cannot corrupt an answer.**
  direnv's own log lines reach stderr, measured, but nothing stops a project's
  `.envrc` printing to stdout and only this repository's has been fixed not to.
  Capturing once confines that text to the one call whose parser tolerates it,
  rather than to every JSON answer bd gives.
- **A directory that cannot be entered fails that project, visibly.** There is
  no fallback to the ambient environment: a mechanism that silently does
  nothing is indistinguishable from one that worked.
- **`credential_command` survives as the escape hatch**, for a tracker outside
  direnv's reach. The config stores a command, never a secret; its stdout is
  the password. What went is its promotion to the default, and the rule that
  demanded one from every project once a second was named. A project naming
  it beside `environment = "direnv"` is refused: it is entered one way, and a
  precedence between the two would be a mechanism nothing on the screen says.
- **An authentication failure is distinguished from the others.**
  `TrackerState::Unreachable` carries a reason: `auth`, `unavailable`, `exec`,
  `parse`, or `unknown-flag`. They want different responses and reporting them
  as one string does not help anyone. The last is bd refusing the command
  line before it runs, in cobra's words (`unknown flag`, `unknown shorthand
  flag`, `unknown command`): a bd older than a flag `bdi` uses, which is what
  a bd below README's floor looks like, and the screen names the floor. Only
  bd's own refusal counts: a credential command or direnv saying the same
  words to a flag it lacks is a configured command that failed, as before,
  and not a bd to replace.
  `bdi` never asks `bd --version`: bd already says which flag it lacks on the
  first call, a version gate cannot see a newer bd that drops a flag, and the
  refusal needs no parsing where a version string would.
- **No error text reaches the output verbatim.** bd's failures name the database
  and user; the reason is reported, the raw stderr is not.

An earlier draft rejected `direnv exec` on two guesses, and both were wrong:
that it costs a direnv evaluation per call, and that it requires every tracker
to be a direnv-managed checkout. The first is answered by capturing once; on
the second, a directory with no `.envrc` runs anyway. Rejecting it as the
*default* was right for a third reason neither guess named: a machine without
direnv got `exec` on its first run and drew nothing, when README had said bd
was all it needed.

A single read-only user across every tracker would retire `credential_command`
entirely, and the shape it would take has been measured — see *Open, for
Graeme* in `CLAUDE.md`. It needs each project's consent, so the design does not
depend on it.

### Degradation is the rule either way

**A root that cannot be read must degrade, not disappear**: it renders where
its row would have been, named and marked with the reason it would not read,
and the live panes recovered for its project render on that project's own
line. A root shown without its beads beats a root silently missing — the same
principle as the default filter.

### Bead ids are not unique across trackers

Each tracker sets its own id prefix and no one coordinates them, so two trackers
can collide. `bdi` reads several trackers in one process, which makes this its
problem in a way it is not for a single-tracker tool. **The key is (project,
id), never id alone.**

## Refresh: when a tracker is read, and who says so

Three things ask for a project to be read again, and they are one mechanism:
the poll a project arms for itself, a message on the inbound channel naming
it, and `^R`, which names every project. Each takes the same window and the
same queue; none has a path of its own, so nothing one of them does can be
lost where the others are kept. A collection runs on a worker thread behind a
channel, never on the loop's — the loop waits on events, one of which happens
to be "the timer fired", and a slow collection still lets the reader scroll.
herdr is polled and stays polled: it has no notification source, it is a local
process, and one `herdr agent list` is nothing beside a project's bd traffic.

**The poll.** A project asks to be read again `refresh_seconds` after its last
read *finished*, whichever of the three asked for that read. The default is
30 seconds, and it governs the fallback timer only: agent and bead state moves
on the order of minutes, `^R` covers impatience, and the two mechanisms below
remove most of what a poll costs. Because each project's next read is timed
from its own last one, projects drift apart rather than all paying the cascade
on the same tick, and a slow project delays only itself. Setting it below a
collection is allowed and bounded, because the gap does not start until the
read ends.

### Reading a tracker only when it has changed

Almost every poll finds nothing has moved. So a refresh asks the tracker
whether it has before asking it anything else: one `bd sql --json "SELECT
dolt_hashof_db() AS h"` for the **Dolt working root**, which hashes everything
the database holds, committed or not — including the wisps, which live in
`dolt_ignore`d tables and never move the committed head. A project whose root
is where the last successful read left it is done there, and its freshness is
as good as if the cascade had run: a skipped read is a successful read, and
the project line says so. A project whose root has moved is read in full.

Measured 2026-09-01 against this project's tracker with bd 1.2.2: the probe
0.203–0.205 s against 1.53 s for the cascade; a read does not move the root
(three probes with a full `bd list --all` and a `bd query` between them
returned the same hash); a wisp write moves the working root and leaves the
head identical, which is why the head was rejected as the probe; an ordinary
bead write moves both.

Three things it has to get right. The root is stored only after the cascade
that followed it succeeded, or a failed read would be sticky. A tracker that
cannot answer the probe — a SQLite-backed one has no `dolt_hashof_db` — gets
the cascade, never "nothing changed": degrade, never disappear. Within that,
a tracker that *refuses* the probe is told from a server that did not answer
it, because the two want different next moves. bd's default store is its
embedded Dolt, and `bd sql` there is refused with `'bd sql' is not yet
supported in embedded mode` on every bd from 1.0.4 to 1.2.2 (measured
2026-09-02): the adapter remembers that refusal per project for the run, and
the tracker is read in full from then on with no probe process in front of
it. A server that did not answer is asked again next refresh, so an outage
never costs the fast path once the server is back. And where a
producer's message arrives for a project whose root has not moved, the probe
wins and the cascade is skipped: bd commits before it returns and a wrapper
pings after, so a real write has already moved the root by the time the
message lands, and an unmoved root means the write was a no-op or the producer
was wrong.

`bd sql` is the one subcommand `--readonly` does not veto, so the no-writes
guarantee at that call site rests on `bdi`'s own string literal rather than on
bd — a constant nothing composes, reached from one function that takes no
argument.

### Telling `bdi` a project changed

`bdi` listens as well as polls. Anything that already knows a tracker changed
can say so, and the project it names is read then rather than at its next
interval. `bdi` ships the socket and the protocol; what produces for it is the
setup's business, and deliberately none of `bdi`'s. A built-in watcher for bd,
Dolt or git is exactly the coupling to one setup's organisation this project
forbids — and the one real producer measured, a Dolt binlog consumer, needs a
replication user and a server-unique id that a DB-scoped tenant cannot have,
and receives every tenant's rows on one stream. An interface fits every setup:
a Dolt trigger, a git hook, a bd wrapper, a systemd path unit, a cron comparing
a head hash, someone typing the line.

**The socket.** `$XDG_RUNTIME_DIR/beady-eye/changes.sock`, a stream socket
created mode `0600`. Under the runtime directory it is user-scoped: it needs no
privilege to create and no other user can reach it. `bdi` removes it when it
exits, and reclaims a stale one left by a run that crashed. A unix socket
rather than a signal because the message must carry *which* project changed —
`bdi` watches several and refreshing all of them throws away the saving — and
rather than a FIFO because a FIFO handles several writers badly.

**The protocol.** Send the name of a project whose work has moved, as one UTF-8
line ending in `\n`. `bdi` answers each line with one line of its own:

| answer | meaning |
|---|---|
| `ok <project>` | a project `bdi` watches. That project is read again; no other is |
| `unknown <project>` | not a project this `bdi` is reading. Nothing happens |
| `malformed` | blank, or longer than 512 bytes. Nothing happens |

The name must match a project's `name` in the config. A connection may carry as
many lines as you like and may stay open for the life of the writer, so a
long-running producer connects once and speaks whenever it has something to
say. `bdi` never initiates; it only answers. The answer goes back to the writer
rather than onto the screen because the writer is the only one who can fix a
wrong name — the person running `bdi` is watching a forest, not a log. A
malformed or unknown message is reported and dropped without disturbing the
loop or the other sources; it arrives from outside `bdi` and must not be able
to take the view down.

**No config key says which projects have a producer, and that absence is the
design.** `bdi` cannot know in advance which projects something reports for,
and a key saying so would be exactly the coupling to a setup's organisation
this bead exists to avoid. Instead every project starts polled; one something
reports for has its poll stood down while messages keep arriving inside the
refresh interval, because each message's read pushes the next poll out past
the interval before it arrives; and a producer going quiet lets the next
interval find the project uncovered, so the poll resumes — the view degrades to
slow, never to stale. Selection and fallback are one mechanism with nothing to
tune. A project whose producer you trust can turn its poll off with `poll =
false`, which is a claim rather than a saving: nothing then covers for a
producer that dies, and an automatic fallback would hide the failure you need
to see. `--poll` and `--no-poll` override every project for one run.

**A socket that cannot be opened is said twice, deliberately, and the two are
not copies.** No `XDG_RUNTIME_DIR`, or another `bdi` already listening, and
this one polls everything exactly as it did before. The notice at the foot
says what it costs the reader — *nothing can tell bdi a project changed ·
every project is polled instead*, or *another bdi held the inbound channel*
where that is the cause, since that one names a process the reader can close.
The `stderr` line names the path and the `io::Error` under it, which is the
actionable half and precisely the half no phrase may carry on screen; it is
written before the alternate screen opens, so it survives the teardown and is
still on the primary screen when the view tears down, and it can be redirected
to a file where a status bar never can. A reader seeing an `eprintln!` beside
a status-bar notice should not delete either. The socket is asked for once and
never again, so a run that started without it goes on polling even after the
path comes free — closing the other `bdi` frees the channel for the next run,
not for this one, and the notice is worded in the tense of the refusal for
that reason.

`README.md` carries the worked example: a `bd` wrapper that writes the project
name to the socket after any command that wrote something.

## The JSON contract

`bdi --json` emits the model, one whole collection.

```json
{
  "generated_at": "2026-08-30T10:22:14Z",
  "herdr": "ok",
  "filter": "live-agents",
  "trees": [
    {
      "project": "summit-works",
      "root": "nix-9670s",
      "title": "Switch the thinkpad's session shell from DMS to noctalia v5",
      "counts": { "total": 21, "closed": 8, "live_agents": 3, "anomalies": 3 },
      "tracker": "ok",
      "nodes": [
        {
          "id": "nix-9670s.20",
          "title": "the daily wallpaper timer calls dms",
          "status": "blocked",
          "issue_type": "task",
          "priority": 2,
          "depth": 1,
          "edge": "parent-child",
          "ready": false,
          "blocked_by": ["nix-9670s.13"],
          "started_at": "2026-08-29T10:00:00Z",
          "closed_at": null,
          "badges": [{ "key": "blocked_on", "text": "⏸ waiting" }],
          "agent": {
            "pane": "wCM:p9",
            "pane_status": "working",
            "title": "shell selector + stable path",
            "source": "agent_pane"
          },
          "anomalies": []
        },
        {
          "id": "nix-9670s.16",
          "title": "guard a key in both layers",
          "status": "in_progress",
          "issue_type": "task",
          "priority": 2,
          "depth": 1,
          "edge": "parent-child",
          "ready": false,
          "blocked_by": [],
          "started_at": "2026-08-29T10:00:00Z",
          "closed_at": null,
          "badges": [],
          "agent": null,
          "anomalies": [{ "rule": "orphan-claim" }]
        }
      ],
      "dangling": [],
      "cycles": []
    }
  ],
  "hidden_trees": [ { "project": "summit-works", "root": "nix-bgej6", "title": "…", "reason": "no-live-agent" } ],
  "failed_projects": [ { "project": "homelab", "tracker": "auth" } ],
  "unattributed": [ { "pane": "wCM:pD", "project": "summit-works", "cwd": "/tmp/bdi-ground/summit-works", "pane_status": "blocked" } ],
  "unconfigured": [ { "pane": "wCM:pF", "cwd": "/srv/spike", "pane_status": "idle" } ],
  "conflicts": []
}
```

`nodes` is pre-flattened in render order with an explicit `depth`, so a consumer
draws it without reconstructing the tree; `edge` says which kind of edge put
the node where it is (`parent-child` or `blocks`), and a bead reachable more
than once is in `nodes` once per way down to it. That is the tree unrolled,
written at emission from a model that holds each bead once (see *Tree
construction*), so what `--json` says does not follow what the model stores. `agent.source` records which
direction of the join resolved it, so a consumer can tell a confirmed agent
from an inferred one. `anomalies` is every rule that fired, `[]` where none
did — never absent, never null; an `orphan-claim` the join refused carries the
refusing conflict as `refused`, and one it did not omits the field. `herdr` is
`ok` or `unavailable`, so a consumer knows which tier it is reading. A tree's
`tracker` is `ok`, `{ "unreachable": <reason> }` where its tracker could not
be read, or `root-not-found` where the tracker answered and holds no bead of
that id — which only a root named in config or on the command line can be,
since every other root came out of the tracker's own answers. `dangling` and
`cycles` name ids that are still in `nodes`. `hidden_trees` is never
empty-by-omission — a filtered tree is reported, not dropped. `failed_projects`
names each project whose tracker could not be read at all, with the reason.

`unattributed` and `unconfigured` are the two ways a live pane resolves to no
bead, and a consumer tells them apart by the `project` key: an `unattributed`
entry always carries it, an `unconfigured` entry never does. The absence is the
contract, so test for the key rather than reading a null.

## TUI

One scrollable forest. Each project owns a line, and the roots drawn for it
hang under it as ordinary bead rows. The selected bead's pane tails below, and
the foot of the screen carries the keys and every notice.

A project's line says what only a project can answer — which project, how
fresh its rows are, how much work it holds, and, where a root would not read,
the live panes still found working there. Everything else is a bead's, and a
root is a bead: its status, its agent, its anomalies and its pane are drawn and
reached exactly as any other row's are, on a row at depth one under the
project. A count on a project line is over its trees with each bead counted
once, because a bead standing in several of them is still one bead. A project
rests open: the forest is what is being worked, and a project shut over it
says only that it exists.

```
▾ summit-works  ✓ 9s ago                             8/21  3 agents  ⚠ 3
  └── ◐ nix-9670s  DMS → noctalia v5                 8/21  ◍ shell selector · working
      ├── ● .20  wallpaper timer calls dms                 ◍ rebuilt generation 541 · working
      ├── ◐ .1   wire the niri theme include               ◍ wCM:p6 · idle · inferred, not confirmed
      │   ├── ○ .4   restore app theming                   1/4
      │   │   ├── ○ .8   make the switch permanent
      │   │   │   └── ○ .9   confirm quickshell wedges gone
      │   │   └── ✓ .5   retire the DMS remnants
      │   └── ○ .17  apply the two niri settings
      ├┄┄ ◐ .16  guard a key in both layers               ⚠ claimed · no pane
      ├─▸ ✓ .3   land the session shell                   2/9  3 unfinished beads beneath this
      └─▸ ✓ 13 more beads · closed, and nobody on them

▾ homelab  ⠋ 1m ago                                  2/7   1 agent
  ├─▸ ◐ hl-sgqyv  heartbeat cadence                  2/7   ◍ pinning the cadence · idle
  └── ⚠ hl-9d2c   the tracker refused the credential it was given

▾ ⚠ 1 pane in a directory no configured project covers
  └── ◍ wCM:pF idle                                  /srv/spike
▸ 4 trees with no live agent                         a to show all
▾ ⚠ 2 unattributed panes
  ├── ◍ wCM:pD waiting at a prompt                   /tmp/bdi-ground/summit-works
  └── ◍ wCM:pE idle                                  /tmp/bdi-ground/summit-works
────────────────────────────────── wCM:p9 ──────────────────────────────────
  · rebuilt .#thinkpad, generation 541
⚠ no herdr session · which agents are alive is unknown   Enter show   a all   ? keys   q quit
```

(The last line shows a notice and the keys together for the sake of the
example; a screen with a herdr session carries no such notice.)

### Every line, and its columns

**The prefix is two columns, then four a level of depth**, and a line resting
shut says so inside its own elbow — `├─▸ `, `└─▸ `, or `├┄▸ ` for a blocker —
so the fold state costs no width and every line at a depth starts in the same
column. A project line at depth zero carries the bare two-column fold marker,
`▾ ` or `▸ `; a group line the same; a root is an ordinary row on the six-column
elbow of any first-level child. A shut marker in the arm is the whole of the
convention — a project, a group and a shut node say so; an open node says it by
drawing its children, and spending a marker on it would only cost the row
width. A root whose tracker never answered has no row to hang anything on, so
it draws its reason where its row would have been and carries no fold.

**The glyph is the bead's own status and nothing else**, glyph for glyph with
the legend at the foot of `bd list`'s own output: `○` open, `◐` in_progress,
`●` blocked, `✓` closed, `❄` deferred, and `?` for a status bd has no legend
for — which the row then quotes, *a status bdi does not recognise: “…”*,
rather than swallowing. Liveness has its own cell, and one glyph meaning both
would make neither readable. The colours are `bd`'s own 24-bit values for the
glyph, literal because `bd`'s are and do not move with the terminal's theme
(read off bd 1.2.2); `open` gets no colour at all, because `bd` sends no
escape for it and inheriting is what lets a row's own brightness reach its
glyph.

**An id is shown as what it adds to its root's**, `.20` for `nix-9670s.20`,
and kept whole where it does not carry that prefix followed by a dot — the
dangling and re-parented nodes, and the root itself — because a bare suffix
would place it under a root it does not belong to.

**Three tiers of brightness say how live a row is**, which is the one thing
about a bead `bd list` has no way to know and so the one thing this scale is
spent on: an agent on it keeps the terminal's default foreground; nobody on it
and still going drops to the theme's colour 8; finished and unworked takes the
grey `bd` dims a closed row to, and a run dims with the rows it stands for.
The scale runs down from the default rather than up from it because a theme's
default is already the brightest thing on its page and nothing can sit above
it: the first version painted a staffed row `White`, and on a theme where
`color15` and the foreground are the same hex — the one it was built on — the
top two tiers were one. Colour 8 is the rung every theme sets and few rows
otherwise use, so the middle tier follows the reader's theme rather than a
hardcoded grey; the cost, accepted, is that every ordinary row dims. Finished
here means what it means to a run — closed, no agent, no anomaly — so a closed
bead whose pane is still alive keeps its brightness, because that is exactly
the row worth looking at. The box-drawing is held at the terminal's default
while the row around it moves: it says how the tree is shaped, not how a bead
is going, and `bd` leaves its own tree prefix undimmed on a closed row too.
Colour is never the only channel: the glyph says the status and the words say
the rest, so a terminal with no colour loses nothing.

**Priority and issue type are not drawn**, and not for want of columns. A row
has exactly two colour-carrying channels — the glyph carries the status,
matching `bd`, and the row's own text carries how live it is — and priority as
a hue wants the second one. The two cannot share it: a P1 bead with an agent on
it would be either `bd`'s orange or the terminal's default, and whichever won,
the other fact would be gone. Liveness is the one only `bdi` can draw.

**The state block, right-aligned, in this order:** the fraction where the line
stands for more than itself; the agent; the anomalies; then, on a line shut
over such things, *◍ 2 agents beneath* and *⚠ 1 bead beneath*; then the notes
— a subtree the tracker stopped at, unfinished work the line is shut over, a
status outside bd's set. A row's own two come before the counts because those
name one bead and these count several: a number met before the name it belongs
beside reads as the total the name is an example of.

**The agent cell says what the agent is doing, not which pane it sits in:**

    ◍ <caption, else pane id> · <state> · <join caveat, where inferred>

The caption is the pane's state label for the state it is in, falling back to
its title — not `display_agent`, which seats set to their own bead id and which
would repeat the row two columns to its left. herdr shows a pane id nowhere a
reader can look one up (only in its socket API and its JSON), so an id here
spends the row's widest cell on a handle nobody can follow. A pane carrying
neither label nor title keeps its id; there it is not noise, it is the only
thing left telling one live agent from another. The separator is the row's own
`·`: a pane id was a token and parsed on sight, but a caption is free text and
*waiting at a prompt* is three words, so run together they would read as one
sentence. The state stays between the caption and the caveat, because the
caveat says how `bdi` knows which pane this is — *inferred, not confirmed*,
where only the pane's `display_agent` named the bead — and against the caption
it would read as doubt about the work. The pane id is demoted, not gone: it
still names the tail's own rule, where the reader has already chosen one pane,
and every conflict carries it in its own fields. herdr's state words are read
verbatim except `blocked`, which is a TTY prompt waiting and one of three
things this tool calls blocked, so it is said in full: *waiting at a prompt*.

**A line that stands for more than itself says how much of that is done** —
the fraction rule in *Tree construction*. A shut closed line over unfinished
work says how many beads, in words, beside the fraction saying it in
arithmetic; a shut line over agents or anomalies counts them.

### The tail

The band under the forest is a rule with the selected pane's id centred in it,
over up to six lines of that pane's output indented two columns; the newest
lines are the ones kept, because a pane's last line is what it is doing now. The
rule is drawn whether or not there is a pane, so the band never goes blank and
always says where the forest stopped. Where there is no pane, the reason sits
under the rule in dim, and there are five: no herdr session; the selection is
not a bead (a project line, a group, or a thing in one); nobody is working this
bead; the pane has gone; the pane is too busy to be read. While a pane is being
read and has not answered, the band says so rather than staying quiet.

Reads use `herdr agent read --source visible`: every agent worth tailing is
alternate-screen and working, and herdr refuses `recent` for those, naming
`visible` as the way through. Nothing the reader is sitting in front of waits
on the read: asking is a send to herdr's own threads — one runs herdr and
blocks in it for as long as herdr takes, the other waits two seconds for that
answer and gives up on it — and the answer arrives later on the loop's channel,
carrying the pane it is about, so an answer about a pane the reader has since
left is dropped. That is what keeps `q` and `^C` answered while a read is
still outstanding. The two seconds bound what the band says, not the loop. One
question at a time: a question that outstays its welcome is not abandoned but
its answer is thrown away when it finally arrives, and none is asked while one
is still out, so a wedged herdr costs one waiting thread rather than one per
poll.

`f` focuses the selected pane in herdr — the only write `bdi` performs, and
it writes to herdr rather than to any system of record. On a row with no pane
it is a no-op, not an error: there is nothing to focus and nothing has gone
wrong. On a pane that will not come, the tail says so where the tail is. From
the bead view, `Enter` does the same — see *The bead*.

`y` puts the selected bead's id on the clipboard — the id alone, exactly as
`bd` takes it — and the foot says *copied bdi-2bb.42* until the reader's next
key or click, because nothing else on the screen changes for it. It is written
with OSC 52, the terminal's own escape sequence for a clipboard write, and with
nothing else: the sequence travels through herdr and ssh the way the rest of
`bdi`'s output does and needs no program outside it, where `wl-copy` or `xclip`
would be a new one with a new seam. A terminal that does not honour OSC 52
drops the sequence, so there the key does nothing — and the foot still says
*copied*, because `bdi` cannot tell. That is the degrade this accepts. On a row
that is not a bead — a project's line, a group, the hidden-trees line — `y`
does nothing and says nothing, as `Enter` does.

The band yields its rows before the forest yields any: on a short screen the
forest is the thing this tool exists to show.

### The bead

`Enter` on a bead row shows that bead whole, as `bd show` would: its glyph,
id and title; its status, priority, type and owner; the agent the join put on
it, with its pane and state; then, under the names `bd show` prints and in its
order, the description, the notes, the parent, what it depends on and what it
blocks — each related bead with its glyph, id and title, and one the tracker's
answer no longer holds named by its id alone and said to be *not in the
tracker's answer*. A section the bead has nothing in is left out, as `bd show`
leaves it out. Everything drawn comes from the rows `bdi` already holds: the
description and the notes are in the `bd list --json` rows, and the related
beads' statuses and titles, with the reverse edge that says what a bead
blocks, are read once over the whole answer when a project is collected. No
key costs a call to `bd`.

It is a window over the forest, like the key bindings, rather than a screen in
place of it: the row it was opened from is untouched beneath it, so leaving
the view puts the reader back on the same row with the forest exactly as they
left it, and a collection landing behind it refreshes the forest and leaves
the view up — unless it moved the selection off the bead, because the bead
closed into a run or left the tracker, in which case the view goes back to the
forest rather than show the forebear the selection fell to under the title the
reader opened. The window follows the terminal: four fifths of the screen on
either side, centred, so a bigger terminal gets a bigger window rather than
the same box in the middle of a bigger forest, and never less than eighty
columns inside its border — about where `bd show` wraps its own prose — by
twenty-four rows, so a small screen is filled rather than cramped. A bead
shorter than that height keeps a window its own height. The description and
the notes are rendered as markdown, in the window's own styling rather than
the forest's: a heading bold and clear of the prose, an item behind `bd`'s own
bullet and hanging under its text, a code span or block in a tone of its own,
emphasis italic and strong emphasis bold, a quote barred down its side, and a
link followed by where it goes. A line break in the source stays a row break,
so text that is not markdown draws as it always did, and text that is broken
markdown is drawn as written: a renderer that drops text is worse than none.
Prose wraps to the window; every other row — the bead's own line, a related
bead's — is cut to it the way a row of the forest is. Where the bead is taller than the window,
the title says how to see the rest, and the motion keys move the bead rather
than the selection: `j`, `k` and the arrows a row, `^D` and `^U` half the
window, `g` and `G` to either end, and the wheel a row a notch.

The view is the hub. From it, `Enter` and `f` focus the bead's pane in herdr,
`y` copies its id, and the view stays up; `Esc` goes back to the forest, and
so does `q`, as it does from the bindings, so the forest a reader was looking
at is still there to quit from. *Back* rather than *close*, because closing is
what `bd close`
does to a bead and this does nothing to one. `?` puts the bindings up over the
forest, `^R` collects behind the view, and every other key does nothing there.
The title — *`<id>` · Esc to go back* — is the line that survives every cut,
because a reader who cannot see how to leave is stuck in a view they may have
opened by accident.

`Enter` on a row that is not a bead — a project's line, a group, a thing in
one, a root whose tree would not read — has nothing to show, and does nothing
and says nothing, as before.

Until this view, `Enter` focused the pane. Graeme moved focus to `f` (15:05
BST 2026-09-02) so that `Enter` shows the bead and focus stays one key from
the row for anyone who learns `f`; the forty-column rule below is why `f` is
in `?` and not on the keys row.

### The groups below the trees

Five, in this order — severity first, and the mock's last two kept in the
mock's order: projects whose tracker could not be read at all; panes in
directories no configured project covers; conflicts nothing could settle; trees
the live-agent filter is holding back; unattributed panes. An empty group draws
nothing. Each line carries its count, so folding a group never loses what it
holds, and each opens to name its members: a failed project with its reason, a
conflict in full, a hidden tree by project and root with its title, a pane by
id and state with the directory it is working in — the directory being what
both pane groups are asking the reader to look at, one to place the agent and
the other to configure the project.

The groups rest by the same rule as the trees. A count is not a view of what it
holds, so a group over live panes — unconfigured, conflicts, unattributed —
rests open; a group that reports on the reading rather than on work in flight —
failed projects, hidden trees — rests shut, and hidden trees in particular
holds trees hidden *because* nothing live is in them, so opening it would
contradict the rule it exists to serve. Hidden trees is the one group nothing
went wrong in — the filter put them there and a key takes them back out — so it
is the only one drawn without a warning, and the only line on the screen that
names a key: *a to show all*. A group that said only how many trees it hides
would read as "nothing to see here" while hiding broken ones, so it also says
how many of them carry findings: *4 trees with no live agent · 1 with
findings*. The findings stay hidden — the reader asked for that — but the group
admits they exist.

Per-tree findings are not groups: a tree's dangling beads, its cycles and the
nodes the tracker stopped at are drawn as note lines directly under its root's
row, whether that root is folded or not, so folding the root cannot lose one.

**Live panes in a project whose tracker could not be read** move out of the
unattributed group onto that project's own line, so every loose pane is on
screen exactly once and the two counts add up. The line says when that list
may be short: a pane under no configured project could belong here and cannot
be told, so one of those anywhere leaves every such recovery marked *and
possibly more*.

### The filter is the reader's, and survives a refresh

`a` toggles between the trees with a live agent and every tree. It is a
state of the view, not a fold: a refresh carries it onto the new snapshot
exactly as it carries the folds and the cursor, so the hidden-trees group a
reader opened by pressing `a` does not shut itself again thirty seconds later.
It was doing so — the filter lived on the snapshot a refresh replaced wholesale
— and because the group's own line is the only place on the screen that names
a key, losing the answer to it read as the group shutting itself. Carrying it
cannot resurrect a filter herdr has made meaningless, because with no herdr
there is no filter to apply and every tree renders.

### Keys

The row under the tail names the handful of bindings worth a permanent line,
each by a key a reader can press — `Enter show   a all   ? keys   q quit`,
thirty-six columns, which is what fits a forty-column terminal without
losing its last words, and the last words are `q quit`. `f focus` beside
`Enter show` would overrun that, so `f` lives in `?` and not on the row. `?`
opens the full table in a window over the forest; any key closes it. `^R`
came off the row to
make room for `?`: refresh is the most skippable of the five, since `bdi`
collects on a timer and on change reports anyway, so `^R` only ever means
*now*, and `?` is one key from the full list.

The bindings are vim-like, with the arrows as aliases:

| key | does |
|---|---|
| `Enter` | show the selected bead, or focus its pane from the bead view |
| `f` | focus the selected bead's pane in herdr |
| `Space` | fold or unfold the selected node |
| `a` | show every tree, not only those with a live agent |
| `?` | show these key bindings |
| `q`, `^C` | quit |
| `Esc` | go back to the forest from the bead view |
| `^R` | collect from the trackers again now |
| `E` | expand the selected node and everything under it |
| `C` | collapse the selected node and everything under it |
| `D` | restore the default view |
| `y` | copy the selected bead's id to the clipboard |
| `Down`, `j` | move down one row |
| `Up`, `k` | move up one row |
| `Right`, `l` | expand, or move to the first child when it is already expanded |
| `Left`, `h` | collapse, or move to the parent when it is already collapsed |
| `^D`, `PgDn` | move down half a screen |
| `^U`, `PgUp` | move up half a screen |
| `Home`, `g` | move to the first row |
| `End`, `G` | move to the last row |

The mapping, the `?` window and the row under the tail are one table read
three ways, so a key is written down once and nothing on screen can disagree
with what pressing it does; a build-time check refuses an action no key
reaches. The table is ordered least guessable first, not naturally, because a
screen too short for the whole of it shows the top: ordered naturally, an
eight-row window gave a reader six motion keys and *8 more* — the arrows, which
they would have pressed anyway. Ordered this way they get `Enter`, `f`,
`Space`, `a`, `?` and a count. A reader who cannot see the arrows presses one
regardless; one who cannot see `a` never works out that the trees they are
missing are being filtered. A window too short for every binding counts the
ones it left off rather than stopping, and the title — *press any key to
close* — is the line that survives every cut, because a reader who cannot see
how to leave is stuck in a view they may have opened by accident.

`h` and `l` carry two meanings each because that is what a tree makes natural
and what every vim-flavoured file tree does; `h` always meaning "parent" would
strand a reader on a collapsed node with no way to open it from the home row.
`E`, `C` and `D` are single keys rather than vim's `zR`, `zM` and `zx`: a
prefix is a mode, and `bdi` has nowhere to say it is in one — vim puts the
pending command in its last line, and `bdi`'s equivalent is the keys row,
which has no spare column. `C` is the one place a reader may knowingly fold
over a live agent, and `D` brings it back: restoring the default recomputes
the spine to live work from the snapshot in hand rather than replaying a
stored fold set, so it stays right after a refresh has changed who is working.

Control held down still moves a row: a key that does not ask for control
answers whatever modifiers are held, which is what the arrows and the letters
have always done, and only `^D`, `^U`, `^R` and `^C` ask for it. That is
inherited rather than chosen, and if it should change it is its own bead.

`^R` is a notification like any other: it takes the same window and the same
queue as a message on the inbound channel and the poll a project arms for
itself, and differs only in naming every project rather than one. It has no
path of its own, so nothing it does can be lost where the other two are kept.

The pointer works too. A click selects the row under it; a wheel notch moves the
selection one row, because `bdi` holds no scroll of its own and the window is
wherever the selection puts it. A click on the tail, on the key row, or on a
blank row past the last line selects nothing, and neither does one on a row the
keyboard cannot rest on — a note. Sliding to the neighbour would select
something the reader did not point at.

**Capture is on for the whole session, unconditionally, and that is a decision
with a cost.** While `bdi` is up the terminal stops getting the mouse, so
dragging over the window no longer selects text in it — a real loss in a tool
whose job is showing bead ids and pane ids you then want to paste. It is taken
anyway, because the stack this is read in gives most of it straight back: herdr
owns the mouse above the pane and keeps its copy mode, which selects by
keyboard, and kitty maps shift-drag to plain text selection even while an
application has grabbed the mouse — starting a selection, and picking out a word
or a line, all still work under shift. What is actually given up is dragging to
select inside one pane, and shift-clicking to extend a selection already made,
which is the one gesture kitty leaves behind when an application grabs. There is
no setting for it: a flag would put the question to every reader when it has one
answer here, and the answer is a property of the terminal rather than of the
reader's taste.

Of everything capture then reports, only those two gestures are answered.
crossterm asks the terminal for any-event tracking, so it reports every cell the
pointer crosses whether a button is down or not. A release, a drag, bare motion,
the other two buttons and the horizontal wheel are each dropped on the thread
that reads them, before the loop can be handed one — a loop wedged by a flood is
a `^C` that never reaches the Quit mapping and a terminal left in raw mode.

### Freshness, beside each project's name

Each project's line says how fresh its own rows are, directly beside its name:
a one-column mark, then how long ago the rows were read. It is per project
because each project is read on its own clock — a refresh naming one project
re-reads that one and leaves every other's rows exactly as they were — and an
indicator that spoke for the whole screen had to quote the *oldest* read to
stay true of every row. That under-promise was the cost of the position, not a
rule worth keeping; beside the name it speaks for that project's rows alone,
so it is exact.

**The mark** is `⠋` turning while a read of the project is outstanding — ten
braille frames at 80 ms, cut from the wall clock rather than counted so every
redraw inside one collection agrees which frame it is (a keystroke redraws the
screen too, and a counter would make the mark jump for it); `⠿`, the turning
mark held still, when the read has been outstanding longer than one may be and
has produced nothing (*unanswered*); `✓` when the last collection read every
root; `⚠` when it met a root it could not read, resolved to the worse where a
project's roots disagree, because the rows under the name are then short of
that root's and a project folded shut draws no other line saying so. One
column in every state, so the cell does not change width for a read starting
or ending.

**The age** is a duration, not a time of day — `9s ago`, `1m ago`, `1h ago`,
`1d ago` — in one unit, the coarsest that still says it. The reader's question
is how much to trust the rows and they answer it from the order of magnitude;
a wall clock made them subtract one time from another. It is said while a
collection runs as well as at rest, because the rows on the screen during a
collection are the previous collection's rows and this is the only thing that
says so. Nothing at all before the first collection of a project comes back:
there is no read to date the rows to, and no rows either. A read that failed
counts as a read — its trees went down with the tracker that refused, so none
of its rows are on screen to be stale — and so does a read the probe found
nothing to do for.

**The cell goes in the line's title block, so it is the first thing a narrowing
line gives up**, and it is given up whole rather than cut — half an age names
no duration. At forty columns the project line is what it always was.

**The mark turns because the loop gives itself a deadline, and so do the
ages.** A collection reports nothing until it is done, and a `bdi` whose
projects are all reported over the inbound channel polls nothing, so an age
left alone would say `0s ago` for as long as the reader left it. So the loop
waits with a deadline for exactly as long as what is drawn is going to stop
being true — one frame while a collection runs, otherwise the next boundary of
the newest read's own unit — and with no deadline at all where nothing on the
screen can go stale. Both halves are cut from a clock by dividing it, so both
hold to that clock's own boundary rather than for a whole unit from whenever
they were last drawn. That deadline is measured from the instant the frame was
drawn at, with one instant handed to both the paint and the wait; two reads of
the clock straddling the moment a read became unanswered once left the loop
waiting with no deadline at all.

**Startup draws the forest first.** Every configured project is on the screen
with its mark turning before any tracker has answered — measured at `cfcbd80`
on the same pty harness and the same three projects: 8.2 s to the first byte
before, 22 ms after — and each fills in as its own collection returns. The
projects come from the config, so the first frame has real content to draw.

### The foot of the screen

The foot is one row: the keys, and every notice the view carries. **A notice
is a fact that has no row to sit on.** Two qualify: a herdr that could not be
reached empties the agent column on every row, and a `bdi` whose inbound
socket would not open is told nothing when a project changes, so the whole
view is only as fresh as the refresh interval. Neither has a row that is
wrong, which is why neither can be said anywhere else. The rule's other edge
is that a per-project fact never belongs there: it has a project line, and the
line is where the reader is already looking at the thing it is about — a
tracker that refused a credential is drawn on its own project, and freshness
moved off the foot for the same reason once it became per project. The foot is
one line, so a per-project fact there is a fact that evicts other facts.

**The order is by consequence, and the foot gives up the last first.** The
keys yield before any notice, because a key can be rediscovered and a fact
silently absent from the one row nothing can fold or scroll away is a fact the
reader never learns. Among the notices, a herdr nobody can reach comes first,
because the agent column is what the reader came for. Where the screen is too
narrow even for the notices in full, words are given up before facts, and from
the end: each phrase has a brief form — *agents unknown*, *polled, not
reported*, *another bdi had it* — and the notice the foot puts first keeps its
full phrase longest. Being cut is the one thing a notice must not be: the mark
a cut leaves is the mark any long line gets, so a severed warning reads as a
sentence that ran out of room rather than as a fact the reader has lost.

A notice reaches the screen by the same road whether a collection produced it
(herdr unreachable, read off the snapshot behind each frame) or this process
did, once at startup (the socket). The status bar is handed a list in the
order it should give them up, so the next fact of this kind needs no new path
to the screen.

## Alternatives considered

**`bv` (beads_viewer)** — a mature Go TUI for beads with a list/detail split, a
kanban board, a dependency view, a multi-repo workspace mode and
PageRank/critical-path insights. Ruled out on three grounds.

**It cannot read a Dolt-backed tracker.** Its readers are SQLite-file and
JSONL-file only; the backend switch has no Dolt arm. Where it detects a Dolt
workspace it shells out to `bd export -o .beads/issues.jsonl` and reads the
file. Against a Dolt server it therefore renders a snapshot of whenever the last
export ran, and this tool's question is what is happening now.

**Its licence is not usable.** MIT plus a rider, declared to control over any
conflicting MIT term, naming OpenAI and Anthropic as Restricted Parties along
with anyone "acting on their behalf, for their benefit, or under their
direction". It grants such parties no rights at all, defines "Use" to include
analysing and benchmarking, must be carried forward unmodified into any
derivative, and terminates automatically on breach.

**It does not take contributions.** Its README states outright that outside PRs
are not merged, so upstreaming is not available either.

Two things it teaches, worth having without the code:

- **Its tree duplicates a node with multiple parents**, materialising a full copy
  of the subtree per parent, and its id→node map keeps whichever copy was built
  last — so navigating by id resolves to only one of the visible instances. An
  earlier draft read that as the case for dedup in the model; `bdi` draws a
  copy per path too, and what it learned instead is that navigation must be
  keyed on the way down to a copy, never on the id (see *Tree construction*).
- **Root-scoping has a direction, and getting it wrong is silent.** It has two
  root-scoped subgraph extractors that disagree: one walks forward from the root
  along dependencies, the other backward from blocker to dependent. Only the
  backward one yields "the root and everything beneath it". Same distinction as
  `bd dep tree --direction=up`, and easy to get backwards while producing a
  plausible-looking tree.

## Optional: wiring an agent workflow into `bdi`

Nothing here is required. This is what a setup gains by adopting two
conventions, and both are one line each.

1. **An agent sets its pane's `display_agent` to the bead id it is working on.**
   `bdi` then resolves that pane to that bead.
2. **An agent writes the reverse key when it claims a bead:**
   ```bash
   bd update <id> --set-metadata agent_pane=$HERDR_PANE_ID
   ```
   `bdi` then detects drift exactly rather than by inference.

Beyond that, a workflow that encodes state in bead metadata — a review link, a
waiting marker, a topic name — surfaces it by naming those keys in `bdi`'s
`[[badges]]` and `[roots]` config. `bdi` gains no knowledge of the workflow; it
draws what it is told to draw.

## Risks

- **The credential boundary** is the one thing that could make the multi-project
  view not work as designed. It is the first thing to prove.
- **`display_agent` is free text.** Any string an agent stamps lands there.
  Parsing it as a bead id is a heuristic; the reverse key is the reliable path,
  which is why both directions exist.
- **bd's CLI is the interface.** `--json` shapes can change. Pin the bd version
  the parser is written against and fail loudly on an unexpected shape, rather
  than rendering a silently wrong tree.
- **The `stale-claim` threshold is a guess.** A long-running bead trips it. A
  warning, never a verdict, and configurable.
- **`unattributed` noise.** Every interactive session shows up here. If it is
  louder than it is useful, it becomes opt-in.
