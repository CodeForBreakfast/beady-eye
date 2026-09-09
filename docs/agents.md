# What `bdi` asks of an agent

`bdi` draws a [beads](https://github.com/gastownhall/beads) tracker's work as
trees and, beside each bead an agent has claimed, the
[herdr](https://herdr.dev) pane that agent sits in. beads is the issue tracker
the work lives in, and `bd` is its command; a bead is one issue. herdr is the
terminal multiplexer you are running under, a pane is the terminal you are in,
and its id is in your environment as `$HERDR_PANE_ID`. `bdi` never writes to a
tracker, so it can only draw what you have written. This page is addressed to
you, the agent: fold its lines into whatever instructions you already follow.

## Name your pane on the bead, and the bead on your pane

`bdi` learns which pane is yours from two things you write. When you start on
a bead, write your pane id onto it and the bead id onto your pane:

```bash
bd update <id> --set-metadata agent_pane="$HERDR_PANE_ID"
herdr pane report-metadata "$HERDR_PANE_ID" --source herdr:claude --display-agent <id>
```

The first line is the join `bdi` trusts: the key names your pane exactly. The
second is the fallback: where the key was never written, `bdi` reads the pane's
`display_agent`, and when that is exactly a bead id in the pane's own project,
it resolves to that bead. The bare id, nothing around it, or the fallback
matches nothing.

`agent_pane` is the default name for the key. A project whose `bdi` config sets
`pane_key` under `[join]` uses that name instead, and it is the one to write.

## One pane joins one bead

Where two beads name one pane, `bdi` awards the key's claim to neither. The
refusal is reported as a conflict, and nothing errors. What the pane says about
itself then stands alone: a `display_agent` naming exactly one of those beads
keeps that bead's agent, and one naming nothing drops your pane among the
unattributed panes below the trees, reading exactly like an agent that never
registered. Working on two beads at once, name your pane on one of them.

## Clear the key when you stop

Clear the key whenever you stop working on the bead, whatever its status:

```bash
bd update <id> --unset-metadata agent_pane
```

Your pane id outlives you and is reassigned to whoever comes next in it. A bead
still naming it then contests the pane with that agent's bead: the key is
refused on both, and the join rests on what the pane says about itself.
`bd close` takes no metadata flags, so the unset goes on `bd update` before it.

## What `bdi` flags

Two rows are drawn as anomalies from the facts above alone.

- `stale-pane`: a bead is closed and a pane still joins to it. Clearing the key
  and the pane's `display_agent` ends it.
- `stale-claim`: a bead is `in_progress` and nothing has written to it for
  thirty days. `bdi` reads the bead's `updated_at`, and any `bd update` moves
  it: a status, a metadata key. The window is `stale_claim_days` under
  `[anomalies]`.

## Anything else goes in metadata

Whatever else you want visible on the row — a review link, a topic name, a
waiting marker — goes in bead metadata under a key of your choosing. The reader
draws it with a `[[badges]]` entry naming that key.
[`[[badges]]`](configuration.md#badges) has the shape.

## Tell `bdi` you changed something

`bdi` polls, and a poll can be half a minute away. After a write, name the
project on `bdi`'s socket and it reads now:
[Telling `bdi` a project changed](configuration.md#telling-bdi-a-project-changed).
