# beady-eye

A view of work in flight: a tree of beads, each node annotated with the live
agent working on it. Every question `bdi` asks a tracker is a read — it shows
you the work, and changing it stays bd's job. That is a promise about the
questions rather than about your tracker: bd writes on its own account on the
way to answering one, and [What it needs](#what-it-needs) says when.

[beads](https://github.com/gastownhall/beads) knows the work — the tree, the
dependency edges, each bead's status and who claimed it.
[herdr](https://herdr.dev) knows the agents — which pane is alive and what it is
doing. Nothing joins them, so there is no single answer to "what is left, what is
done, and what is being worked on right now" — and drift between the two (a
closed bead whose agent never exited, a claim whose agent died) is invisible.

`bdi` is that join. The work is the spine; agents are an annotation.

```
▾ atlas  ✓ 29s ago                                                   3/12  3 agents  ⚠ 1
  ├── ○ atlas-1   Payments move to the new gateway                                   2/9
  │   ├── ◐ atlas-3   The refund path calls the gateway          0/3  ◍ wE3:pE · working
  │   │   ├── ◐ atlas-5   Retire the old refund worker               ⚠ claimed · no pane
  │   │   └── ○ atlas-4   Backfill the refund ledger
  │   ├── ◐ atlas-7   Webhook retries are not idempotent              ◍ wE3:pD · working
  │   ├── ○ atlas-2   Pin the gateway client version
  │   ├── ○ atlas-6   Cut the live keys over                                         0/2
  │   │   └┄┄ ○ atlas-2   Pin the gateway client version
  │   ├── ✓ atlas-8   Reconcile the settlement report
  │   └── ✓ atlas-9   Drop the gateway shim
  ├── ○ atlas-10  Search returns stale results after an edit                         1/3
  │   ├── ◐ atlas-11  Invalidate the index on write                   ◍ wE3:pF · working
  │   └── ✓ atlas-12  Measure the reindex cost
  └── ⚠ 3 unattributed panes
────────────────────────────────────────────────────────────────────────────────────────
  no pane · nobody is working this bead





Enter show   a all   ? keys   q quit
```

That is a real run at 88 columns, against a throwaway tracker holding made-up
work and joined to the panes that were actually alive on the machine that took
it. `atlas-5` is the drift: a claim with nothing behind it.

## What it needs

**bd 1.1.0 or newer.** That floor is about the command line `bdi` runs, and a
bd below it is told apart from a tracker that cannot answer: the project's line
says bd does not know a flag `bdi` uses, and which bd would. A bd older than
the one that last wrote a tracker is a separate hazard, on the tracker itself.

**One bd per tracker, which is yours to arrange and not `bdi`'s.** `bdi` never
spells a subcommand that writes, and `--readonly` is on every line it spells
that names a tracker — but neither settles what bd does on its way to
answering. bd rewrites `.beads/.local_version` and runs its schema
auto-migration on finding itself newer than the bd that last opened that
tracker, before the subcommand runs and whatever the subcommand is;
`--readonly` does not stop it, because the flag vetoes bd's own mutating
subcommands and never reaches the storage layer.

Note what that trigger compares: the bd running now against the bd that ran
last, not one installed bd against another. Upgrading your only bd arms it too,
on the first read after the upgrade — so one bd per tracker does not avoid the
event, and nothing short of never upgrading would. What it buys is that a
tracker moves forward once, at an upgrade you chose, rather than being carried
somewhere by a bd that is not that project's. `bdi` links no bd — it runs
whatever each project's environment resolves, which is why the arrangement is
yours.

Nothing marks the moment. bd suppresses its upgrade notice under `--json`, on
both streams, so the read comes back clean and `bdi` has nothing to draw; and
because the migration leaves the tracker at the new version, every read after
it is quiet too. `docs/design.md` carries the measurements.

**git, where you have it.** `bdi` asks git for three things, and does without
each: the repository the current directory sits in, the name of its `origin`
remote, and the working trees a project has. With no git at all a run still
reads its tracker. A project a config names keeps the path it was configured
with and, having no working trees to place a pane by, holds only that; a
project found without a config is the tree the beads workspace sits at the top
of, and takes that directory's name rather than the remote's.

**herdr is optional.** Without it you still get the trees, the counts and the
claims, and with no agents to filter on every tree is drawn whatever the filter
asked for. What herdr adds is the annotation this is all for: which claim has a
live pane behind it, which pane is working somewhere no bead accounts for, and
a tail of the selected bead's pane.

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

## Configuring it

`bdi` reads `~/.config/beady-eye/config.toml`, or whatever file `--config`
names, and goes on reading it: a config you edit while `bdi` is running takes
effect a couple of seconds later, on every setting. One that will not parse
leaves the config in force exactly as it was and says so at the foot until you
fix it.

With no config file at all, `bdi` reads the one project the directory it was
started in belongs to. Where git can be asked, that is the repository the
directory is in, named after its `origin` remote — or after the repository's
own directory, where the repository has no `origin`.

Where git cannot be run at all, the project is the tree the beads workspace bd
found sits at the top of, named after that directory — and the directory `bdi`
was started in, where that workspace is not one beads made above it. So the
project is the same one wherever inside it you started, but it is named after
a directory rather than after a remote — and two machines that cloned one
repository into differently-named directories call it different things.

`BDI_PROJECT` in the environment names the project instead of any of that, and
is how one name holds on every machine. That is the only variable `bdi` reads
for a name, so a shell that keeps the project's name in another tool's variable
exports it under this one too.

So a first run inside a repository beads tracks needs no config at all. A
first run anywhere else has nothing to fall back on and says so:

```console
$ bdi
Error: there is no config at /home/you/.config/beady-eye/config.toml, so bdi read the current directory

Caused by:
    /home/you is not in anything beads tracks
```

A config is a list of projects, and everything else in it has a default:

```toml
[[projects]]
name = "atlas"
path = "/home/you/atlas"

[[projects]]
name = "orbital"
path = "/srv/work/orbital"
environment = "direnv"

[[projects]]
name = "beacon"
path = "/home/you/dev/beacon"
credential_command = "secret-tool lookup tracker beacon"

[roots.explicit]
atlas = ["atlas-1", "atlas-10"]

[[badges]]
key    = "delivery_pr"
render = "⇢ {}"

[[badges]]
key    = "blocked_on"
match  = "human"
render = "⏸ waiting"

[join]
pane_key = "agent_pane"

[anomalies]
stale_claim_days = 30

[tui]
refresh_seconds = 30
unanswered_after_seconds = 30
tail_refresh_millis = 250

[theme]
background = "light"
```

**`[[projects]]`** is the one section with no default. A project is a `name`
and the `path` its repository is at; the name is how `bdi` tells one tracker's
beads from another's, so two projects cannot answer to one.

Each project's tracker is read in the environment `bdi` itself was started in,
so a tracker your shell can already reach needs nothing configured. A setup
that keeps one credential per project in each project's own directory, loaded
by direnv when you enter it, says `environment = "direnv"`, and that project is
then read with what entering its directory produces, at the cost of one
`direnv exec` per refresh. direnv is worth naming when the password bd needs is
in a project's `.envrc` and nowhere in the shell running `bdi`; a single
tracker, or a SQLite one, wants the default. `credential_command` is the third
way in, for a tracker outside both: a command whose stdout is the password. A
project names one of the three.

**`[roots.explicit]`** names trees to draw beyond the ones `bdi` finds for
itself, listed under the project whose tracker holds each. Bead prefixes are
per-tracker and uncoordinated, so an id on its own names nothing `bdi` can go
and read.

**`[[badges]]`** draws one metadata key beside every bead carrying it. `render`
is the text, with `{}` standing for the value; `match` narrows the badge to one
value, and a badge with no `match` draws whatever the key holds. `bdi` knows
nothing about any particular way of organising agents — no orchestration model,
no roles, no workflow — so a convention your setup encodes in bead metadata is
named here and drawn without interpretation.

**`[join] pane_key`** is the metadata key holding the pane an agent sits in.
That is what ties an agent to its bead exactly, rather than inferring it from
what the pane calls itself.

**`[anomalies]`** sets how many days a claim may go untouched before `bdi`
calls it stale. The default matches `bd stale --days`, so it is beads' own
window rather than a number `bdi` invented.

**`[tui]`** sets three clocks, all of them gaps after an answer rather than
periods a read happens inside, so a slow tracker stretches its own gap instead
of piling asks up behind itself. `refresh_seconds` is how long a project waits
after one read before asking for the next; `unanswered_after_seconds` is how
long a read may be outstanding before the screen says the tracker has stopped
answering rather than drawing it as merely being read; `tail_refresh_millis` is
how often the tail asks herdr for the selected pane again, in milliseconds
because it is the one interval under a second.

**`[theme] background`** is the one thing about your terminal `bdi` neither
sees nor asks about, so a reader on a light background says it here. Absent,
it assumes `dark`. That is a guess, and where it shows is the tail band: a
light-background reader who leaves the key unset stops being able to tell
`bdi`'s own words there from the pane's output around them. `dark` and `light`
are the only values, and anything else is refused rather than read as the
default:

```console
$ bdi
Error: unknown variant `Light`, expected `dark` or `light`
in `theme.background`
```

## Running it

A bare `bdi` draws the forest and keeps it live. The screen is three bands:
the scrollable forest, with a line per project and its trees hanging under it;
the tail beneath, headed by the selected bead's pane and showing that pane's
last rows; and one row at the foot carrying every notice on the left and the
keys on the right.

Move with the arrow keys or `hjkl`. Four keys do most of the rest:
`Enter` opens the selected bead, `f` brings its pane to the front, `a` shows
every tree rather than only the ones with a live agent, and `^R` collects from
the trackers again now. `y` puts the selected bead's id on the clipboard,
written as OSC 52: no helper program, and it travels through a multiplexer and
ssh the way the rest of the screen does. A terminal that does not honour the
sequence drops it, so on one of those the key does nothing rather than failing.
`q` quits, and `?` lists every binding there is in a window over the forest.

`--json` writes the same snapshot to stdout instead of drawing it. A `bdi`
whose stdout is not a terminal has nowhere to draw and says so:

```console
$ bdi | cat
bdi's view needs a terminal; re-run with --json
```

## Which projects a run reads

The directory `bdi` is started in decides. Started under one of the projects
the config names — its own directory, a repository inside it, or a linked
worktree of it — `bdi` reads that project and no other, and says so on the
screen. Started outside every configured project it reads all of them. The
projects left out are never read, rather than read and hidden, and the config
still knows them: a pane working in one is placed in its own project, not
reported as somewhere `bdi` was never told about.

`--all-projects` reads every configured project wherever `bdi` is started.
`--project <NAME>`, repeated for more than one, reads only those, from
anywhere, and outranks the directory. Naming a bead as `<project>:<bead-id>`
adds its tree to the run; from a directory that chose a different project, it
reads that project too. Against an explicit `--project` that left the project
out it is refused instead — one command line asking for a project's tree and
asking not to read that project contradicts itself.

## Telling `bdi` a project changed

`bdi` polls, and almost every poll finds nothing has moved. So a refresh asks
the tracker whether it has, before asking it anything else: one `bd sql` for
the Dolt working root, which covers everything the database holds including
the ephemeral beads that are never committed. A project whose root is where
the last read left it is done there. Only a Dolt server can answer that
probe: bd's default store, its embedded Dolt, refuses it, and `bdi` takes the
refusal as the answer for the rest of the run rather than asking again on
every refresh.

A project whose root has moved is read in full, and that is the cascade the
poll used to run every time: `bd` for what is ready, what is blocked, the
ephemeral beads and every bead there is — four `bd` invocations, with the
roots to draw under read off the last two. The environment its tracker is
read in is settled before any of them, and is a process of its own only
where the project named direnv or a credential command. So the whole refresh
is that cascade plus the probe, plus that process where there is one, most of
it against a remote Dolt server.

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
| `unknown <project>` | Not a project this `bdi` is reading. Nothing happens. |
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

Built, unreleased, and in daily use against the trackers it was written for.
There is no tag yet, so the install commands above are what the first one makes
work. The design is in [docs/design.md](docs/design.md), reconciled against what
got built.
