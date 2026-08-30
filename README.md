# beady-eye

A read-only view of work in flight: a tree of beads, each node annotated with the
live agent working on it.

[beads](https://github.com/steveyegge/beads) knows the work — the tree, the
dependency edges, each bead's status and who claimed it.
[herdr](https://herdr.dev) knows the agents — which pane is alive and what it is
doing. Nothing joins them, so there is no single answer to "what is left, what is
done, and what is being worked on right now" — and drift between the two (a
closed bead whose agent never exited, a claim whose agent died) is invisible.

`bdi` is that join. The work is the spine; agents are an annotation.

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
```

## What it assumes

**bd, and nothing else.** herdr is optional: without it you still get the tree,
the counts, the claims and an age-based stale-claim warning. With it you also get
liveness, exact drift detection, and a tail of the selected bead's pane.

It knows nothing about any particular way of organising agents — no orchestration
model, no roles, no workflow. Conventions your setup encodes in bead metadata are
named in config and drawn as badges; `bdi` never learns what they mean.

## Telling `bdi` a project changed

`bdi` polls, and almost every poll is wasted. One refresh of one project runs a
credential command, discovery, `bd ready`, `bd blocked_by`, a `parent_of` per
discovered bead and a `dep_tree` per root — dozens of round trips to a remote
Dolt server, repeated for every project, and nearly always finding nothing has
moved.

So `bdi` listens. Anything that already knows a tracker changed can say so, and
the project it names stops being polled for as long as it keeps saying it.

**The socket.** `$XDG_RUNTIME_DIR/beady-eye/changes.sock`, a stream socket
created mode `0600`. Under the runtime directory it is user-scoped: it needs no
privilege to create and no other user can reach it. `bdi` removes it when it
exits, and reclaims a stale one left by a run that crashed.

**The protocol.** Send the name of a project whose work has moved, as one UTF-8
line ending in `\n`. `bdi` answers each line with one line of its own:

| Answer | Meaning |
| --- | --- |
| `ok <project>` | A project `bdi` watches. That project is read again; no other is. |
| `unknown <project>` | Not a project this `bdi` was configured with. Nothing happens. |
| `malformed` | Blank, or longer than 512 bytes. Nothing happens. |

The name must match a project's `name` in the config. A connection may carry as
many lines as you like and may stay open for the life of the writer, so a
long-running producer connects once and speaks whenever it has something to say.
`bdi` never initiates; it only answers.

The answer goes back to the writer rather than onto the screen because the
writer is the only one who can fix a wrong name — the person running `bdi` is
watching a forest, not a log. A writer that does not care can ignore it.

**What a message does to the poll.** Nothing tells `bdi` in advance which
projects have a producer, so it works it out from what arrives. Every project
starts polled. A project something reports for stops being polled for as long as
messages keep arriving inside the refresh interval. A poll reads only the
projects it still has to find, so a project with no producer costs nothing to
the projects that have one. If the producer goes away, the next interval finds
the project uncovered and the poll resumes — the view degrades to slow, never
to stale. Nothing needs configuring for any of this, and
a project nobody wires up simply carries on being polled.

If the socket cannot be opened at all — no `XDG_RUNTIME_DIR`, another `bdi`
already listening — `bdi` says so on stderr as it starts and polls everything,
exactly as it did before.

**A worked example.** The cheapest producer is the thing already making the
changes. Wrap `bd` so that a command which wrote something tells `bdi` about it:

```bash
bdi_changed() {
  local sock="$XDG_RUNTIME_DIR/beady-eye/changes.sock"
  [ -S "$sock" ] || return 0
  printf '%s\n' "$1" | socat - UNIX-CONNECT:"$sock" >/dev/null 2>&1
}

bd() {
  command bd "$@" || return
  case "$1" in
    create|update|close|note|dep) bdi_changed my-project ;;
  esac
}
```

`bdi` closes its end as soon as the writer closes theirs, so that round trip
costs milliseconds and is not worth backgrounding. `nc -N -U "$sock"` does the
same job where you have OpenBSD netcat rather than socat. To check by hand what
`bdi` makes of a name:

```console
$ printf 'my-project\n' | socat - UNIX-CONNECT:"$XDG_RUNTIME_DIR/beady-eye/changes.sock"
ok my-project
```

Anything else that knows works as well and `bdi` cannot tell the difference: a
Dolt trigger, a git hook, a systemd path unit, a cron job comparing a head hash,
a replication-stream consumer, or you typing the line yourself. `bdi` ships the
socket and the protocol; what produces for it is your setup's business, and
deliberately none of `bdi`'s.

## Status

Design accepted, not yet implemented. See [docs/design.md](docs/design.md).
