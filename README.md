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

## Install

Two channels, both first class. Either gives you the `bdi` command, built from
the same commit at the same version — a release tag produces both or neither.

Nothing is published yet, so these are what the first tag makes work.

**crates.io**

```console
$ cargo install beady-eye
```

The crate is `beady-eye`, after the repository; the command it installs is
`bdi`.

**Nix**

Run it without installing anything:

```console
$ nix run github:CodeForBreakfast/beady-eye
```

Or keep it:

```console
$ nix profile install github:CodeForBreakfast/beady-eye
```

To build it into your own flake, take it as an input pinned to a release tag:

```nix
inputs.beady-eye.url = "github:CodeForBreakfast/beady-eye/v0.1.0";
```

That gives you two ways in. Reach the package directly:

```nix
beady-eye.packages.${system}.default
```

or add the overlay, after which `pkgs.beady-eye` is the package anywhere you
have a `pkgs`:

```nix
nixpkgs.overlays = [ beady-eye.overlays.default ];
```

## What it assumes

**bd, and nothing else.** herdr is optional: without it you still get the tree,
the counts, the claims and an age-based stale-claim warning. With it you also get
liveness, exact drift detection, and a tail of the selected bead's pane.

It knows nothing about any particular way of organising agents — no orchestration
model, no roles, no workflow. Conventions your setup encodes in bead metadata are
named in config and drawn as badges; `bdi` never learns what they mean.

## Telling `bdi` a project changed

`bdi` polls, and almost every poll finds nothing has moved. So a refresh asks
the tracker whether it has, before asking it anything else: one `bd sql` for
the Dolt working root, which covers everything the database holds including
the ephemeral beads that are never committed. A project whose root is where
the last read left it is done there.

A project whose root has moved is read in full, and that is the cascade the
poll used to run every time: `bd` for the beads discovery starts from, what is
ready, what is blocked, the ephemeral beads and every bead there is — seven
`bd` invocations, plus one per metadata key roots are discovered by and one
per bead the climb to a root steps onto that discovery did not name. The
environment its tracker is read in is captured before any of them, so the
whole refresh is those seven plus the probe plus that capture, most of it
against a remote Dolt server.

So the poll is cheap, and `bdi` listens as well. Anything that already knows a
tracker changed can say so, and the project it names is read then rather than
at its next interval.

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

**What a message does to the poll.** A project asks to be read again one
`refresh_seconds` after its last read *finished*, whichever of the three
things asked for that read — a message, the poll, or `^R`. So a project
something keeps reporting for is never polled: each message's read pushes the
next poll out past the interval before it arrives. A project whose producer
goes away comes due one interval after its last read and is polled from then
on — the view degrades to slow, never to stale. Nothing needs configuring for
any of this, and a project nobody wires up simply carries on being polled.

Because each project's next read is timed from its own last one, projects
drift apart rather than all paying the cascade on the same tick, and a slow
project delays only itself.

**Turning the poll off.** A project whose producer you trust can stop polling
altogether:

```toml
[[projects]]
name = "atlas"
path = "/home/you/atlas"
poll = false
```

That is a claim rather than a saving: nothing then covers for a producer that
dies, which is the point — an automatic fallback would hide the failure you
need to see. `bdi` polls until told otherwise, so a project that says nothing
is polled.

For one run, `--poll` polls every project whatever the config says and
`--no-poll` polls none, which is how to find out whether a suspect producer
was the only thing wrong without redeploying anything.

If the socket cannot be opened at all — no `XDG_RUNTIME_DIR`, another `bdi`
already listening — `bdi` says so on stderr as it starts and polls everything,
exactly as it did before. The socket is asked for once and never again, so a
run that started without it goes on polling even after the path comes free —
closing the other `bdi` frees the channel for the next run, not for this one.

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
