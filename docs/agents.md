# What `bdi` asks of an agent

`bdi` draws a tracker's beads as trees and, beside each bead an agent has
claimed, the herdr pane that agent sits in. It never writes to a tracker, so
it can only draw what you have written. This page is addressed to you, the
agent: fold its lines into whatever instructions you already follow.

## Name your pane on the bead, and the bead on your pane

When you take a bead, write your pane id onto it and the bead id onto your
pane:

```bash
bd update <id> --status in_progress --set-metadata agent_pane="$HERDR_PANE_ID"
herdr pane report-metadata "$HERDR_PANE_ID" --source herdr:claude --display-agent <id>
```

The first line is the join `bdi` trusts: the key names your pane exactly. The
second is the fallback: where the key was never written, `bdi` reads the pane's
`display_agent`, and when that is exactly a bead id in the pane's own project,
it resolves to that bead. Write both. The bare id, nothing around it, or the
fallback matches nothing.

`agent_pane` is the default name for the key. A project whose `bdi` config sets
`pane_key` under `[join]` uses that name instead, and it is the one to write.

## One pane joins one bead

Where two beads name one pane, or two panes name one bead, `bdi` awards the
key's claim to neither. The refusal is reported as a conflict, and nothing
errors. What the pane says about itself then stands alone: a `display_agent`
naming exactly one of those beads keeps that bead's agent, and one naming
nothing drops your pane among the unattributed panes below the trees, reading
exactly like an agent that never registered. Holding two beads, name your pane
on one of them.

## Clear the key on every way out

Clear the key whenever you let the bead go: closing it, handing it back,
leaving it open behind a pull request, or exiting.

```bash
bd update <id> --unset-metadata agent_pane
```

Your pane outlives your seat and is reassigned to whoever comes next. A bead
still naming it then contests the pane with that seat's bead: the key is
refused on both, and the join rests on what the pane says about itself.
`bd close` takes no metadata flags, so the unset goes on `bd update` before it.

## Leave when the bead closes

A closed bead whose pane is still alive is `stale-pane`. Once the bead is
closed, exit.

## Touch the bead as the work moves

An `in_progress` bead nobody has written to for thirty days is `stale-claim`.
`bdi` reads the bead's `updated_at`, and any `bd update` moves it: a status, a
metadata key. The window is `stale_claim_days` under `[anomalies]`.

## Anything else goes in metadata

Whatever else your workflow wants visible on the row — a review link, a topic
name, a waiting marker — goes in bead metadata under a key of your choosing.
The reader draws it with a `[[badges]]` entry naming that key.
[`[[badges]]`](configuration.md#badges) has the shape.

## Tell `bdi` you changed something

`bdi` polls, and a poll can be half a minute away. After a write, name the
project on `bdi`'s socket and it reads now:
[Telling `bdi` a project changed](configuration.md#telling-bdi-a-project-changed).
