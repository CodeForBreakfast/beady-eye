# beady-eye — design

Status: accepted, not yet implemented
Date: 2026-08-30
Binary: `bdi`

## The problem

A fleet effort is driven by a bead graph and executed by ephemeral herdr panes.
Two systems hold the truth and neither holds all of it:

- **bd** knows the work — the anchor epic, its children, the dependency edges,
  each bead's status, and who claimed it.
- **herdr** knows the workers — which pane is alive, what state it is in, and
  (by convention) which bead it stamped into `display_agent`.

Nothing joins them. That costs two things.

**No single view of an effort.** `bd graph` renders the tree but knows nothing
about agents. `herdr agent list` knows the agents but nothing about the tree.
`/fleet-status` rolls up efforts from a commy topic but goes no deeper than one
line each. To answer "what is left, what is done, what is being worked on right
now" you read three surfaces and hold the join in your head.

**Drift between the two is invisible.** Observed live on `nix-9670s` while
writing this design:

| bead | bd says | herdr says |
|---|---|---|
| `.6` | closed | pane `wCM:p4` working |
| `.11` | closed | pane `wCM:pB` working |
| `.16` | in_progress | no pane carries this bead |
| `.20` | `blocked_on=human` | pane `wCM:p9` working |

The first two are seats that finished and never went away. The third is a claim
with no visible worker — or a seat that has not stamped `display_agent` yet, and
**today those two are indistinguishable**. The fourth is a worker that woke on a
relay and never cleared its key.

## Goal

One tree of work per effort, rooted at the anchor epic, with each node annotated
by live agent activity where there is any. The work is the spine; agents are an
annotation that is absent on most rows.

## Non-goals for v1

- **No remediation.** Never writes to bd, never closes a pane. It surfaces
  anomalies; you act in the pane it jumps you to. Two things deciding a bead is
  orphaned is how a live worker's claim gets lost, and the orchestrator already
  owns that decision.
- **No Noctalia widget.** The JSON contract is specified here; no Luau ships
  until the TUI has proved the data model. Noctalia v5 is a beta at
  `plugin_api` v9 — a widget written against it now gets rewritten twice.
- **No cross-machine view.** One herdr session on one box. `herdr --remote` is a
  later consumer of the same collector.

## Architecture

Three units, each testable alone.

```
  bd CLI (per project) ──→  collector  ─→  model  ─→  ┬─→  TUI (ratatui)
  herdr socket (optional) ─┘             (pure)      └─→  --json (serde)
```

**collector** — the only unit that does I/O. Shells out, parses, joins.
**model** — pure functions: tree construction, anomaly rules, ordering, the
default filter. All the logic worth a test lives here and needs neither herdr nor
bd to run.
**renderers** — two consumers of the same model. `--json` is the contract a
future Noctalia widget reads; the TUI is the working surface.

The split exists because the widget must not re-derive anything, and because
Noctalia's plugin API will move under us.

## herdr is a provider, not a dependency

**bd discovers the efforts. herdr filters and enriches them.**

An earlier draft had herdr enumerate the roots. That is wrong, and the evidence
is concrete: `bd list --has-metadata-key working_topic` turns up `nix-bgej6`
(`summit-works/mac-deploy`) alongside `nix-9670s`, and `nix-bgej6` **has no pane
in herdr at all**. Herdr-first enumeration drops it silently — the same failure
as an unreachable tracker, from the other side.

So `bdi` runs in two tiers:

| | bd only | + herdr |
|---|---|---|
| tree of work, correctly drawn | ✅ | ✅ |
| done / left / in-flight counts | ✅ | ✅ |
| who claimed it, and when | ✅ | ✅ |
| waiting-on-you (`blocked_on=human`) | ✅ | ✅ |
| in delivery (`delivery_pr`) | ✅ | ✅ |
| stale claim | by age — a heuristic | exact |
| seat alive right now | ✗ | ✅ |
| `stale-seat`, `woken-uncleared` | ✗ | ✅ |
| pane tail and focus | ✗ | ✅ |

The age-based stale-claim heuristic earns its keep on its own. Right now
`nix-o8hou` has sat `in_progress` since 2026-07-07 and `nix-jg8om` since
2026-07-03 — 54 and 58 days. No herdr needed to see that. It stays a warning
rather than a verdict, because a genuinely long-running bead trips it too.

### The default filter

**When herdr is available, the default view is efforts with at least one live
pane** — the work actually in flight, which is the common question. Efforts with
no seat are not dropped; they collapse to a single line so nothing disappears
silently:

```
▸ 4 efforts with no live seat            a to show all
```

`a` toggles to the unfiltered set. When herdr is unavailable there is no filter
to apply, so every discovered effort renders, with a one-line note that liveness
is unavailable.

## Discovery

Roots come from bd, in this order, unioned and deduped:

1. Beads carrying `working_topic`, `herdr_pane`, `blocked_on` or `delivery_pr`
   — the fleet-native signals — walked to their root epic via `parent-child`
   ancestors.
2. Epics with at least one child that is not closed and at least one child
   `in_progress` or `blocked`.
3. Any bead named by a live pane's `display_agent` that the first two missed.
   This is the only root herdr contributes, and it exists so a seat working
   off-graph still shows up.

## Data sources

Verified against a live session, 2026-08-30.

### bd

`bd dep tree <anchor> --direction=up --json` is the tree source. It:

- walks **both** edge kinds and labels each `parent-child` or `blocks`
- **dedups** — a bead with five blockers appears once, under the first path that
  reaches it (`--show-all-paths` opts out)
- carries id, title, status, priority, and the edge type per node

We follow its model and redo its rendering: the shipped glyphs are wrong (`.14`
draws under `.20`, which is not one of its blockers, and `└──` appears mid-list
where `├──` belongs).

`bd list --has-metadata-key <key> --limit 0 --json` supplies discovery and the
reverse join. `--limit 0` matters — the default is 50 and a truncated list
silently reclassifies beads.

### herdr

`herdr agent list` returns JSON over the session socket. Per agent, the fields
that matter:

| field | use |
|---|---|
| `pane_id` | the join key, e.g. `wCM:p9` |
| `cwd` | resolves the pane to a project root |
| `display_agent` | the bead id a worker stamped, e.g. `nix-9670s.20` |
| `title` | the worker's one-line "what I am doing" |
| `state_labels` | per-state text, e.g. `idle: "asleep: needs eyes on focus-ring colour"` |
| `agent_status` | `idle` / `working` / `blocked` / `done` |
| `workspace_id`, `tab_id` | grouping, and the target for focus |

`herdr agent read <pane>` gives terminal output for the tail pane.
`herdr agent focus <pane>` is the only write v1 performs, and it writes to
herdr, not to any system of record.

**`agent_status: blocked` is not the bead's `blocked_on=human`.** herdr's
`blocked` means a TTY prompt is waiting — an auto-mode permission gate, or a
seat still at the dev-channel confirmation during boot. A bead blocked on Graeme
usually presents as an `idle` pane. The model keeps these in separate fields and
must never conflate them.

There is no event stream. `herdr api snapshot` prints live state; the collector
polls.

## The join

Bidirectional, because each direction alone has a hole.

**pane → bead** is `display_agent`, already written by `/fleet-worker`'s claim
step. Its hole: a seat that has not stamped yet (booting, or one that simply
forgot) carries no bead, so its work looks unstaffed.

**bead → pane** is a new `herdr_pane` metadata key, written next to the
`working_topic` write the claim step already performs:

```bash
bd update <id> --set-metadata herdr_pane=$HERDR_PANE_ID
```

Its hole: a project whose skills have not adopted the key.

Together they close both. A bead is **live** if either direction resolves to a
pane present in `herdr agent list`. The pane id exists from spawn, so there is no
race against a booting seat — which is what makes death detection exact rather
than a guess.

`beady-eye` degrades to `display_agent` alone where `herdr_pane` is absent, and
says so on the affected rows. It never requires the key.

## Anomaly rules

All computed in the pure model. The last three need herdr; the first does not.

| rule | condition | reading |
|---|---|---|
| `aged-claim` | `in_progress`, not updated in N days | claim probably abandoned |
| `orphan-claim` | `in_progress`, no pane resolves for it | worker died mid-claim |
| `stale-seat` | bead is closed, its pane is alive | seat finished and did not self-close |
| `woken-uncleared` | `blocked_on=human`, pane is `working` | worker resumed, never cleared the key |
| `unattributed` | pane alive in a known project, no bead resolves | a seat nobody can account for |

One guard, carried over from `/fleet-orchestrator`: **a bead that is `blocked`
with `blocked_on=human` and whose pane is idle is never an anomaly.** That worker
is alive and waiting on a relay by design. Getting this wrong reads every
sleeping worker as dead.

`unattributed` will also catch interactive sessions and concierge panes, which
are not anomalies at all. It renders as its own collapsed group, not as an error
against any effort.

## Tree construction

1. Discovery (above) yields the effort roots.
2. Per root, `bd dep tree <root> --direction=up --json`.
3. `herdr agent list`, if reachable, is joined onto the nodes.
4. The default filter drops efforts with no live seat to a collapsed count.

Ordering within a level: state first (in-flight, then blocked, then open, then
closed), priority second, id third. Closed subtrees collapse to a count.

## Cross-project credentials — the hard constraint

`bd` runs in server mode against `tracker.example.invalid:3306` with **one database
and one MySQL user per project**, and the password arrives as
`BEADS_DOLT_PASSWORD` from that project's direnv. So a process holding
summit-works's credential cannot read homelab's tracker:

```console
$ bd -C /tmp/bdi-ground/homelab list
Error: failed to open database: ... Error 1045 (28000): Access denied for user 'homelab'
```

bd found homelab's config — it knew the database and the user — and had no
password for it. **This is a credential boundary, not a policy one**, and it is
the single biggest constraint on the multi-project view.

Two ways out, and the design does not have to pick now:

- **`direnv exec <project-root> bd …`** per project, so each query runs in the
  environment that owns its credential. Costs a direnv evaluation per project per
  refresh, which caching makes tolerable. Untested — verifying it needs a seat
  with reach into a second project.
- **A credential set given to `bdi` directly**, one entry per project in its own
  config. Simpler and faster; a second place secrets live, which is a real cost.

Either way, **a tracker that cannot be reached must degrade, not disappear**: the
effort renders as a header with its live panes and a `tracker unreachable`
marker. An effort shown without its tree beats an effort silently missing — the
same principle as the default filter.

## The JSON contract

`bdi --json` emits the model. Shape:

```json
{
  "generated_at": "2026-08-30T10:22:14Z",
  "herdr": "ok",
  "filter": "live-seats",
  "efforts": [
    {
      "project": "summit-works",
      "anchor": "nix-9670s",
      "title": "Switch the thinkpad's session shell from DMS to noctalia v5",
      "counts": { "total": 21, "closed": 8, "live_seats": 3, "anomalies": 3 },
      "tracker": "ok",
      "nodes": [
        {
          "id": "nix-9670s.20",
          "title": "noctalia: the daily wallpaper timer calls dms",
          "status": "blocked",
          "priority": 2,
          "edge": "parent-child",
          "depth": 1,
          "blocked_by": ["nix-9670s.13"],
          "blocked_on": "human",
          "delivery_pr": null,
          "agent": {
            "pane": "wCM:p9",
            "herdr_status": "working",
            "title": "shell selector + stable path",
            "source": "herdr_pane"
          },
          "anomaly": "woken-uncleared"
        }
      ]
    }
  ],
  "hidden_efforts": [ { "project": "summit-works", "anchor": "nix-bgej6", "reason": "no-live-seat" } ],
  "unattributed": [ { "pane": "wCM:pD", "cwd": "/tmp/bdi-ground/summit-works", "herdr_status": "blocked" } ]
}
```

`nodes` is pre-flattened in render order with an explicit `depth`, so a consumer
draws it without reconstructing the tree. `agent.source` records which direction
of the join resolved it, so a consumer can tell a confirmed seat from an inferred
one. `herdr` is `ok` or `unavailable`, so a consumer knows which tier it is
reading. `hidden_efforts` is never empty-by-omission — a filtered effort is
reported, not dropped.

## TUI

One scrollable forest. Every effort is a top-level node, collapsed to its header
by default, expanded on the selected one. The selected bead's pane tails in a
bottom split.

```
▾ summit-works · nix-9670s   DMS → noctalia v5      8/21   3 seats  ⚠ 3
  ├── ● .20  wallpaper timer calls dms            ◍ wCM:p9  working
  ├── ● .1   wire the niri theme include          ◍ wCM:p6  asleep
  │   ├── ○ .4   restore app theming
  │   │   ├── ○ .8   make the switch permanent
  │   │   │   └── ○ .9   confirm quickshell wedges gone
  │   │   └── ○ .5   retire the DMS remnants
  │   └── ○ .17  apply the two niri settings
  ├── ◐ .16  guard a key in both layers          ⚠ claimed · no pane
  └── … 13 more

▸ homelab · hl-sgqyv   heartbeat cadence            2/7    1 seat

▸ 4 efforts with no live seat                             a to show all
▸ ⚠ unattributed                                          2 panes
────────────────────────────────────── wCM:p9 ──────────────────────────────
  · rebuilt .#thinkpad, generation 541
  ⏎ focus   a all   ^R refresh   q quit
```

Keys: arrows to move, space to fold, `⏎` to focus the pane in herdr, `a` to drop
the live-seat filter, `^R` to refresh, `q` to quit. Refresh is a poll — herdr has
no event stream — on a default interval with `^R` to force one.

## Change required outside this repo

One line in `/fleet-worker`'s claim step, alongside the `working_topic` write:

```bash
bd update <id> --set-metadata herdr_pane=$HERDR_PANE_ID
```

Nothing else changes. `beady-eye` works without it, less precisely.

## Risks

- **The credential boundary** is the one thing that could make the multi-project
  view not work as designed. It needs settling before the collector is built, and
  it is the first thing to prove.
- **`display_agent` is free text.** Any string a worker stamps lands there.
  Parsing it as a bead id is a heuristic; `herdr_pane` is the reliable path, which
  is why both directions exist.
- **bd's CLI is the interface.** `--json` shapes can change under us. Pin the bd
  version the parser is written against and fail loudly on an unexpected shape,
  rather than rendering a silently wrong tree.
- **The `aged-claim` threshold is a guess.** A long-running bead trips it. It is a
  warning, never a verdict, and it should be configurable.
- **`unattributed` noise.** Every interactive session shows up here. If it is
  louder than it is useful, it becomes opt-in.
