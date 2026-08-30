# beady-eye — design

Status: accepted, not yet implemented
Date: 2026-08-30
Binary: `bdi`

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
is concrete: `bd list --has-metadata-key working_topic` turned up a root with no
pane in herdr at all. Herdr-first enumeration drops it silently.

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

`a` toggles to the unfiltered set. When herdr is unavailable there is no filter
to apply, so every discovered tree renders, with a one-line note that liveness
is unavailable.

## Discovery

Roots come from bd, unioned and deduped:

1. Beads whose status is `in_progress` or `blocked`, walked to their root via
   `parent-child` ancestors. This uses bd's own statuses and needs no
   convention.
2. Beads carrying any metadata key named in `roots.metadata_keys` (see
   *Conventions are configuration*), likewise walked to their root. Empty by
   default.
3. Roots named explicitly in config, or as `bdi <bead-id>` arguments.
4. Any bead named by a live pane's `display_agent` that the first three missed.
   This is the only root herdr contributes, and it exists so an agent working
   off-tree still appears.

## Conventions are configuration

Different setups encode different things in bead metadata. `bdi` hard-codes none
of them. Its config names which keys to notice:

```toml
[roots]
metadata_keys = ["working_topic"]      # presence marks a bead as live work

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

`bd dep tree <root> --direction=up --json` is the tree source, and it gives more
than expected. It returns a **flat array, already in render order**, one row per
bead, carrying:

| field | use |
|---|---|
| `id`, `title`, `status`, `priority`, `issue_type` | the row |
| `parent_id` | the tree edge — empty on the root |
| `depth` | pre-computed nesting level |
| `edge_from_parent` | `parent-child` or `blocks` |
| `metadata` | the whole map, inline |
| `updated_at`, `started_at`, `closed_at`, `owner`, `assignee` | the age rules |
| `truncated` | bd hit its depth limit on this node |

Three consequences, all simplifications:

- **We do not build the tree.** bd has already resolved it. The model assembles
  rows into a renderable structure by `parent_id` and re-orders siblings; it does
  not walk edges.
- **bd already dedups.** Measured on a 23-node tree: 23 rows, 23 distinct ids,
  none repeated — including a node with four blockers, which appears once.
- **Badges need no second call.** `metadata` is inline per row, so a configured
  key is read from the row that already loaded.

What we still redo is the **rendering**. bd's text tree emits broken glyphs —
vertical connectors missing under a node that has following siblings, and child
indent that does not line up with its parent's marker. The JSON is sound; only
the drawing is not.

Two parsing notes. `bd show <id> --json` returns an **array**, not an object, so
a consumer indexing it as a map fails. And `truncated` must be surfaced rather
than ignored: a truncated node means the tree shown is incomplete, which is
exactly the kind of silent partial answer this tool exists to avoid.

`bd list --has-metadata-key <key> --limit 0 --json` supplies discovery and the
reverse join. `--limit 0` matters — the default is 50, and a truncated list
silently reclassifies beads.

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
| `workspace_id`, `tab_id` | grouping, and the target for focus |

`herdr agent read <pane>` gives terminal output for the tail pane.
`herdr agent focus <pane>` is the only write `bdi` performs, and it writes to
herdr, not to any system of record.

**`agent_status: blocked` means a TTY prompt is waiting** — a permission gate, or
a pane still at a startup confirmation. It is a property of the terminal, not of
the work, and it must never be conflated with a bead's status or with any
configured badge. The model keeps them in separate fields.

There is no event stream. `herdr api snapshot` prints live state; the collector
polls.

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

## Anomaly rules

All computed in the pure model. The first needs bd alone; the rest need herdr.

| rule | condition | reading |
|---|---|---|
| `aged-claim` | `in_progress`, not updated in N days | claim probably abandoned |
| `orphan-claim` | `in_progress`, no pane resolves for it | agent died mid-claim |
| `stale-pane` | bead is closed, its pane is alive | agent finished and did not exit |
| `unattributed` | pane alive in a known project, no bead resolves | a pane nobody can account for |

`orphan-claim` keys on `in_progress` alone. A bead that is `blocked` with a live
pane is not an anomaly — an agent parked on a blocked bead is a normal state,
and firing on it would report every waiting agent as dead.

`unattributed` will also catch ordinary interactive sessions, which are not
anomalies. It renders as its own collapsed group, never as an error against a
tree.

## Tree construction

1. Discovery yields the roots.
2. Per root, `bd dep tree <root> --direction=up --json`.
3. `herdr agent list`, if reachable, is joined onto the nodes.
4. The default filter collapses trees with no live agent to a count.

Ordering within a level: state first (in-flight, then blocked, then open, then
closed), priority second, id third. Closed subtrees collapse to a count.

**Dedup belongs in the model, not the renderer.** A node reachable by several
paths is materialised once, and the id→node map resolves to that one node. The
alternative — a copy per path — makes id-based navigation land on whichever copy
was built last, which is a real bug in an existing viewer (see *Alternatives
considered*).

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

### v1 takes a per-project credential set

`bdi` holds one credential per project it may read, in its own config. Confirmed
with the operator of this deployment: no cross-project reader exists today; the
one read-only user on the server is scoped to a single database.

### A shared read-only user is the clean follow-up, and its shape is measured

Measured on the current server generation (Dolt 2.1.11) with a throwaway
SELECT-only user:

- reads reached the base tables **and the `ready_issues` view** — that view
  carries no `DEFINER` clause, so it resolves with the invoker's privileges
- `CREATE TABLE`, `INSERT`, `UPDATE` and `DELETE` were each refused

The view result is the one that matters for a bead-graph viewer, and it is a
measurement rather than an assumption. A multi-database reader is therefore
`GRANT SELECT ON <db>.*` repeated per tracker against one user — a proven shape,
not new design. It is not v1's problem for a non-technical reason: reading
another project's tracker needs that project's consent.

### Degradation is the rule either way

**A tracker that cannot be reached must degrade, not disappear**: the tree
renders as a header with its live panes and a `tracker unreachable` marker. A
tree shown without its beads beats a tree silently missing — the same principle
as the default filter.

### Bead ids are not unique across trackers

Each tracker sets its own id prefix and no one coordinates them, so two trackers
can collide. `bdi` reads several trackers in one process, which makes this its
problem in a way it is not for a single-tracker tool. **The key is (project,
id), never id alone.**

## The JSON contract

`bdi --json` emits the model.

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
          "priority": 2,
          "edge": "parent-child",
          "depth": 1,
          "blocked_by": ["nix-9670s.13"],
          "badges": [{ "key": "blocked_on", "text": "⏸ waiting" }],
          "agent": {
            "pane": "wCM:p9",
            "pane_status": "working",
            "title": "shell selector + stable path",
            "source": "agent_pane"
          },
          "anomaly": null
        }
      ]
    }
  ],
  "hidden_trees": [ { "project": "summit-works", "root": "nix-bgej6", "reason": "no-live-agent" } ],
  "unattributed": [ { "pane": "wCM:pD", "cwd": "/tmp/bdi-ground/summit-works", "pane_status": "blocked" } ]
}
```

`nodes` is pre-flattened in render order with an explicit `depth`, so a consumer
draws it without reconstructing the tree. `agent.source` records which direction
of the join resolved it, so a consumer can tell a confirmed agent from an
inferred one. `herdr` is `ok` or `unavailable`, so a consumer knows which tier it
is reading. `hidden_trees` is never empty-by-omission — a filtered tree is
reported, not dropped.

## TUI

One scrollable forest. Every root is a top-level node, collapsed to its header
by default, expanded on the selected one. The selected bead's pane tails below.

```
▾ summit-works · nix-9670s   DMS → noctalia v5      8/21   3 agents  ⚠ 3
  ├── ● .20  wallpaper timer calls dms            ◍ wCM:p9  working
  ├── ● .1   wire the niri theme include          ◍ wCM:p6  idle
  │   ├── ○ .4   restore app theming
  │   │   ├── ○ .8   make the switch permanent
  │   │   │   └── ○ .9   confirm quickshell wedges gone
  │   │   └── ○ .5   retire the DMS remnants
  │   └── ○ .17  apply the two niri settings
  ├── ◐ .16  guard a key in both layers          ⚠ claimed · no pane
  └── … 13 more

▸ homelab · hl-sgqyv   heartbeat cadence            2/7    1 agent

▸ 4 trees with no live agent                              a to show all
▸ ⚠ unattributed                                          2 panes
────────────────────────────────────── wCM:p9 ──────────────────────────────
  · rebuilt .#thinkpad, generation 541
  ⏎ focus   a all   ^R refresh   q quit
```

Keys: arrows to move, space to fold, `⏎` to focus the pane, `a` to drop the
live-agent filter, `^R` to refresh, `q` to quit. Refresh is a poll — herdr has no
event stream — on a default interval with `^R` to force one.

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
  last — so navigating by id resolves to only one of the visible instances. That
  is why dedup belongs in the model.
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
- **The `aged-claim` threshold is a guess.** A long-running bead trips it. A
  warning, never a verdict, and configurable.
- **`unattributed` noise.** Every interactive session shows up here. If it is
  louder than it is useful, it becomes opt-in.
