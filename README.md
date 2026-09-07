# beady-eye

`bdi` shows the work in flight across your [beads](https://github.com/gastownhall/beads)
trackers as a tree, with the live [herdr](https://herdr.dev) agent drawn beside
each bead it is working on.

beads knows the work: the tree, the dependencies, each bead's status and who
claimed it. herdr knows the agents: which pane is alive and what it is doing.
Neither knows about the other, so "what is left, what is done, and who is on
what right now" has no single answer — and a closed bead whose agent never
exited, or a claim whose agent died, is invisible to both. `bdi` joins them.

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

A real run at 88 columns, against a throwaway tracker of made-up work joined to
the panes that were alive on the machine. `atlas-5` is the drift: a claim with
nothing behind it.

`bdi` only reads. Changing the work stays bd's job.

## Install

From crates.io:

```console
$ cargo install beady-eye
```

With Nix, run it without installing:

```console
$ nix run github:CodeForBreakfast/beady-eye
```

or keep it:

```console
$ nix profile install github:CodeForBreakfast/beady-eye
```

Neither is a download. Nothing publishes a binary cache for this project, so
the first run compiles it from source and takes minutes. And both follow
`main`, so what they build is the tip of the default branch rather than the
last release.

They also need flakes, and a Nix without them refuses twice — once per feature,
and obeying the first refusal does not clear the second:

```console
$ nix run github:CodeForBreakfast/beady-eye
error: experimental Nix feature 'nix-command' is disabled; add '--extra-experimental-features nix-command' to enable it

$ nix --extra-experimental-features nix-command run github:CodeForBreakfast/beady-eye
error: experimental Nix feature 'flakes' is disabled; add '--extra-experimental-features flakes' to enable it
```

Ask for both at once:

```console
$ NIX_CONFIG='experimental-features = nix-command flakes' nix run github:CodeForBreakfast/beady-eye
```

or write that same line into `nix.conf` and the prefix stops being needed. The
flag nix itself suggests does the same, as long as both features are named at
once — naming them one at a time is the loop above.

To use it from your own flake, pin the input to a release tag — which is how you
get a build you can name afterwards — and take either the package or the
overlay:

```nix
inputs.beady-eye.url = "github:CodeForBreakfast/beady-eye/v0.1.0";

# then either
beady-eye.packages.${system}.default
# or
nixpkgs.overlays = [ beady-eye.overlays.default ];   # pkgs.beady-eye
```

The crate is `beady-eye`; the command it installs is `bdi`.

## Run it

Inside a repository beads tracks, `bdi` needs no config:

```console
$ cd ~/atlas
$ bdi
```

It draws that project's trees and keeps them live. The screen has three bands:
the forest, with a line per project and its trees under it; a tail showing the
last rows of the selected bead's pane; and a foot row with notices on the left
and keys on the right.

These are the keys to get started with; `?` shows every binding there is:

| key | does |
|---|---|
| `↑` `↓`, `j` `k` | move up and down a row |
| `←` `→`, `h` `l` | collapse, or move to the parent when it is already collapsed; expand, or move to the first child when it is already expanded |
| `Enter` | show the selected bead, or focus its pane from the bead view |
| `f` | focus the selected bead's pane |
| `a` | show every tree, not only those with a live agent |
| `Space` | fold or unfold the selected node |
| `E`, `C` | expand or collapse the selected node and everything under it |
| `/`, `n`, `N` | find part of an id or title; next and previous match |
| `y` | copy the selected bead's id to the clipboard (OSC 52, so it works over ssh and through a multiplexer) |
| `^R` | read the trackers again now |
| `?` | every binding |
| `q` | quit |

`bdi --json` writes the same snapshot to stdout instead of drawing it. That is
also what to use when stdout is not a terminal — `bdi | cat` says so and exits.

Outside anything beads tracks, and with no config file, there is nothing to
read:

```console
$ bdi
Error: there is no config at /home/you/.config/beady-eye/config.toml, so bdi read the current directory

Caused by:
    /home/you is not in anything beads tracks
```

### Which projects a run reads

With a config naming several projects, the directory you start in decides.
Under one of them — its directory, a repository inside it, or a linked worktree
— `bdi` reads that project alone and says so on screen. Anywhere else, it reads
all of them.

`--all-projects` reads every configured project from anywhere. `--project NAME`
(repeatable) reads only those, from anywhere. A bead named on the command line
as `PROJECT:ID` adds its tree to the run, and reads that project if the
directory would have left it out.

## Configure it

`bdi` reads `~/.config/beady-eye/config.toml`, or the file `--config` names,
and re-reads it while running: an edit takes effect a couple of seconds later.
A file that does not parse leaves the previous config in force and says so at
the foot until it is fixed.

Everything has a default except the project list:

```toml
[[projects]]
name = "atlas"
path = "/home/you/atlas"

[[projects]]
name = "orbital"
path = "/srv/work/orbital"
environment_command = "nix develop -c"

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

### `[[projects]]`

A `name` and the `path` of its repository. The name is how `bdi` tells one
tracker's beads from another's, so no two projects share one.

Without a config, the one project is named after the repository's `origin`
remote, or its directory if there is no remote or no git. `BDI_PROJECT` in the
environment overrides that name, which is how to keep one name across machines
that cloned into differently-named directories.

**`environment_command`** — the wrapper you would type yourself to enter the
project's environment, if `bdi` cannot work it out. A directory with an
`.envrc`, on a machine with direnv, needs nothing: `bdi` enters it with
`direnv exec .` on its own. Otherwise name the wrapper:

| entered with | write |
|---|---|
| nix | `environment_command = "nix develop -c"` |
| mise | `environment_command = "mise exec --"` |
| direnv, from an `.envrc` somewhere else | `environment_command = "direnv exec ."` |

The command runs in the project's directory. It is split on spaces with no
quoting, so an argument containing a space is written as a list:

```toml
environment_command = ["nix", "develop", ".#dev shell", "-c"]
```

Whichever way it is entered, the tracker is read with the `bd` that environment
supplies — the one you would get by standing in the directory yourself. A
project that asked for an environment `bdi` could not produce is not read at
all, and the screen says so; on a fresh clone that is usually an `.envrc`
waiting for `direnv allow`. See [What it needs](#what-it-needs) for why there is
no fallback.

**`credential_command`** — a command whose stdout is the tracker's password.
It runs inside the project's environment, and its output is captured rather
than passed on a command line, so the password never shows in `ps`.

**`poll = false`** — stop polling this project and rely on something
[telling `bdi` when it changed](#telling-bdi-a-project-changed). Nothing then
covers for a producer that dies, which is deliberate: an automatic fallback
would hide the failure.

### `[roots.explicit]`

Trees to draw beyond the ones `bdi` finds for itself, listed under the project
whose tracker holds them. Bead prefixes are per-tracker, so an id has to be
placed.

### `[[badges]]`

Draw a metadata key beside every bead that carries it. `render` is the text,
with `{}` for the value; `match` restricts the badge to one value. `bdi` has no
idea what your metadata means — a convention your setup encodes there is named
here and drawn as written.

### `[join]`

`pane_key` is the metadata key that names the herdr pane an agent sits in. It
ties an agent to its bead exactly, rather than guessing from what the pane calls
itself.

### `[anomalies]`

`stale_claim_days` is how long a claim may go untouched before `bdi` flags it.
The default is `bd stale`'s own window.

### `[tui]`

Three intervals, each a gap *after* an answer rather than a fixed period, so a
slow tracker stretches its own gap instead of queueing reads behind itself.
`refresh_seconds` is how long a project waits between reads;
`unanswered_after_seconds` is how long a read may take before the screen says
the tracker has stopped answering; `tail_refresh_millis` is how often the tail
asks herdr for the selected pane.

### `[theme]`

`background` is `dark` or `light`. `bdi` cannot see your terminal's background
and assumes `dark`; on a light one the tail band becomes hard to read until you
say so.

## Telling `bdi` a project changed

`bdi` polls, and most polls find nothing moved. A poll first asks the tracker
whether anything has changed (one `bd sql` for the Dolt working root) and only
reads in full if it has. That probe needs a Dolt server; bd's embedded store
refuses it, and `bdi` then reads in full on every poll.

Anything that already knows a tracker changed can skip the wait. `bdi` listens
on `$XDG_RUNTIME_DIR/beady-eye/changes.sock`, a stream socket created mode
`0600` and removed on exit. Write a project's name as one line; `bdi` reads that
project now and answers on the same connection:

| answer | meaning |
|---|---|
| `ok <project>` | read again now |
| `unknown <project>` | not a project this run is reading |
| `malformed` | blank, or over 512 bytes |

A connection can carry as many lines as you like and stay open for as long as
the writer does. A project that is reported for is never polled — each report
pushes the next poll past its interval — and one whose producer goes quiet is
polled again from one interval later. The view degrades to slow, never to stale.

The cheapest producer is a wrapper round `bd` itself:

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

`nc -N -U "$sock"` does the same with OpenBSD netcat. A Dolt trigger, a git
hook, a systemd path unit or a cron job comparing a head hash all work equally
well; `bdi` provides the socket and cannot tell them apart.

`--poll` and `--no-poll` override every project's `poll` setting for one run,
which is how to find out whether a suspect producer was the only thing wrong.

If the socket cannot be opened — no `XDG_RUNTIME_DIR`, or another `bdi` already
listening — `bdi` says so on stderr at startup and polls everything.

## What it needs

**bd 1.1.0 or newer.** An older bd is reported as such on the project's line,
rather than as a tracker that cannot answer.

**Each tracker read by its own bd.** `bdi` never writes to a tracker, but bd
does: on finding itself newer than the bd that last opened a tracker, bd
rewrites `.beads/.local_version` and migrates the schema, before running
whatever subcommand it was given. `--readonly` does not stop that, and under
`--json` bd says nothing about it. So a tracker read with a bd that is not its
project's can be moved to a schema its project's bd cannot open — which is why
`bdi` runs each project's own `bd` through the environment ladder above, and
refuses to fall back to its own when that fails. Upgrading a project's bd
migrates on the first read afterwards; that is the upgrade you chose.
`docs/design.md` has the measurements.

**git, optionally.** `bdi` uses it for the repository a directory sits in, the
`origin` name, and the linked worktrees. Without git it still reads the
tracker, names the project after its directory, and cannot place a pane by
worktree.

**herdr, optionally.** Without it you get the trees, the counts and the claims,
with every tree drawn since there are no agents to filter on. herdr adds the
part this is for: which claim has a live pane behind it, which pane is working
on nothing any bead accounts for, and the tail.

## Status

Released, and in daily use against the trackers it was written for. The design
is in [docs/design.md](docs/design.md).

Versions are `0.x`, and a breaking change bumps the minor: `0.1` → `0.2`. So a
minor bump can break you — pin the input to a release tag, as the example above
does. `1.0.0` is a version the maintainers will choose once the shape has
settled, rather than one a change arrives at by breaking something.
