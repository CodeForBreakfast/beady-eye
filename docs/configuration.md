# Configuring `bdi`

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

[[projects.badges]]
key    = "delivery_pr"
render = "⇢ beacon/{}"

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

[changes]
socket = "/run/user/1000/beady-eye/changes.sock"

[anomalies]
stale_claim_days = 30

[tui]
refresh_seconds = 30
unanswered_after_seconds = 30
tail_refresh_millis = 250
wheel_notch_lines = 3

[theme]
background = "light"
```

## Which projects a run reads

With a config naming several projects, the directory you start in decides.
Under one of them, meaning its directory, a repository inside it, or a linked
worktree, `bdi` reads that project alone and says so on screen. Anywhere else,
it reads all of them.

`--all-projects` reads every configured project from anywhere. `--project NAME`
(repeatable) reads only those, from anywhere. A bead named on the command line
as `PROJECT:ID` adds its tree to the run, and reads that project if the
directory would have left it out.

Without a config, the one project is named after the repository's `origin`
remote, or its directory if there is no remote or no git. `BDI_PROJECT` in the
environment overrides that name, which keeps one name across machines that
cloned into differently-named directories.

## `[[projects]]`

A `name` and the `path` of its repository. The name is how `bdi` tells one
tracker's beads from another's, so no two projects share one.

**`environment_command`** is the wrapper you would type yourself to enter the
project's environment. A directory with an `.envrc`, on a machine with direnv,
needs nothing: `bdi` enters it with `direnv exec .` on its own. Otherwise name
the wrapper:

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

The tracker is read with the `bd` that environment supplies, the one you would
get by standing in the directory yourself. A project whose environment `bdi`
could not produce is not read at all, and the screen says so; on a fresh clone
that is usually an `.envrc` waiting for `direnv allow`. There is no fallback to
some other `bd`, and [Each tracker read by its own bd](#each-tracker-read-by-its-own-bd)
says why.

**`credential_command`** is a command whose stdout is the tracker's password.
It runs inside the project's environment, and its output is captured rather
than passed on a command line, so the password never shows in `ps`.

**`poll = false`** stops polling this project and relies on something
[telling `bdi` when it changed](#telling-bdi-a-project-changed). Nothing then
covers for a producer that dies.

## `[roots.explicit]`

Trees to draw beyond the ones `bdi` finds for itself, listed under the project
whose tracker holds them. Bead prefixes are per-tracker, so an id has to be
placed.

## `[[badges]]`

Draw a metadata key beside every bead that carries it. `render` is the text,
with `{}` for the whole value. `match` restricts the badge to the values a
pattern matches, and the pattern is anchored against the whole value: `human`
draws on `human` and not on `inhumane`.

A capture the pattern names is `render`'s to place by that name:

```toml
[[badges]]
key    = "delivery_pr"
match  = "[^/]+/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ {repo} #{number}"
```

`link` is where the badge points, written as a template over the same captures
`render` reads. A badge that has one is drawn underlined, which is the whole of
what says it is a link — the URL is nowhere in the text on the row.

```toml
[[badges]]
key    = "delivery_pr"
match  = "(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ #{number}"
link   = "https://forge.invalid/{owner}/{repo}/pull/{number}"
```

**A `link` naming something this value did not supply is no link at all.** A
`delivery_pr` written as a bare `12` gives an optional `owner` and `repo`
nothing, and a URL built round the parts that were never there points at
somewhere else. The badge still draws its `render`; it just has nowhere to go.

`bdi` has no idea what your metadata means and draws the badge as written.

## `[[projects.badges]]`

What one project draws in place of this list, for the keys it names and no
others. A shared list can only say what the value itself carries, so a project
whose `delivery_pr` leaves out its owner and repository needs an entry of its
own to supply them. Every key a project stays silent about keeps drawing what
`[[badges]]` says.

It shadows by key rather than by entry: a project naming `blocked_on` replaces
*every* `[[badges]]` entry for `blocked_on`, however many values they match
between them. Its entries stand where the first `[[badges]]` entry for that key
stood, so overriding one badge does not reorder the row.

**A project's own keys have to come before its badges.** `[[projects.badges]]`
opens a table of its own, so `name`, `path` or anything else written after it
belongs to the badge, which refuses it:

```
unknown field `path`, expected one of `key`, `match`, `render`, `link`
in `projects.badges`
```

## `[join]`

`pane_key` is the metadata key that names the herdr pane an agent sits in. It
ties an agent to its bead exactly.

## `[changes]`

`socket` is where `bdi` listens for something saying a project's work has
moved. It defaults to `$XDG_RUNTIME_DIR/beady-eye/changes.sock`, and a machine
with no `$XDG_RUNTIME_DIR` has no channel until this names one. `--socket`
overrides it for one run, which is how two `bdi` runs on one machine each get
a channel. [Telling `bdi` where to listen](#telling-bdi-where-to-listen) has
the whole of it.

## `[anomalies]`

`stale_claim_days` is how long a claim may go untouched before `bdi` flags it.
The default is `bd stale`'s own window.

## `[tui]`

Three intervals, each a gap *after* an answer rather than a fixed period, so a
slow tracker stretches its own gap instead of queueing reads behind itself.
`refresh_seconds` is how long a project waits between reads;
`unanswered_after_seconds` is how long a read may take before the screen says
the tracker has stopped answering; `tail_refresh_millis` is how often the tail
asks herdr for the selected pane.

`wheel_notch_lines` is how far one notch of the wheel moves the tree, and the
bead window over it. The default of three is the terminal convention, and it
suits a wheel mouse, which reports one notch however far the detent turned.
Raise or lower it for a trackpad, which reports as you travel rather than in
detents: the same flick covers three times the ground. Your terminal's own
scroll multiplier cannot help here — a terminal neutralises it while a program
is reading the mouse, so `bdi` sees one report per notch whatever you set it
to.

## `[theme]`

`background` is `dark` or `light`. `bdi` cannot see your terminal's background
and assumes `dark`; on a light one the tail band becomes hard to read until you
say so.

## Telling `bdi` a project changed

`bdi` polls, and most polls find nothing moved. A poll first asks the tracker
whether anything has changed (one `bd sql` for the Dolt working root) and only
reads in full if it has. That probe needs a Dolt server; bd's embedded store
refuses it, and `bdi` then reads in full on every poll.

Anything that already knows a tracker changed can skip the wait. `bdi` listens
on a stream socket, created mode `0600` and removed on exit,
`$XDG_RUNTIME_DIR/beady-eye/changes.sock` unless it is told otherwise. Write a
project's name as one line; `bdi` reads that project now and answers on the
same connection:

| answer | meaning |
|---|---|
| `ok <project>` | read again now |
| `unknown <project>` | not a project this run is reading |
| `malformed` | blank, or over 512 bytes |

A connection can carry as many lines as you like and stay open for as long as
the writer does. A project that is reported for is never polled, since each
report pushes the next poll past its interval, and one whose producer goes
quiet is polled again from one interval later.

The cheapest producer is a wrapper round `bd` itself. It reads the default
path; a `bdi` told a different one has to be told to the producer too.

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

### Telling `bdi` where to listen

The default path is one per login session, so two `bdi` runs on one machine
derive the same one and the second finds the first already listening. It says
so on stderr and polls everything for the rest of its life: the socket is asked
for once at startup and never again. Give one of them a socket of its own and
both have a channel:

```console
$ bdi --socket /run/user/1000/beady-eye/worktree.sock
```

A machine with no `$XDG_RUNTIME_DIR`, and macOS has none, has no path to derive
and no channel until it is told one. It wants the same path every run, so it
belongs in the config:

```toml
[changes]
socket = "/Users/you/Library/Caches/beady-eye/changes.sock"
```

`--socket` overrides the key.

A path you name may sit somewhere other users can walk through. The socket is
created `0600` wherever it goes, so nobody else can speak on it, but who may
*replace* it is for the directories above it to say. `bdi` checks every
directory on the way down, as spelled and as it resolves: each has to be yours
or the system's and closed to everybody else, or sticky. Where one is a
directory somebody else may take a name in, `bdi` names it and polls. A
directory `bdi` creates itself is `0700`, so `/tmp/beady-eye/changes.sock` is
a channel: `/tmp` is sticky and the directory under it is yours.

A path already holding something that is not a socket is refused and left
alone. A socket a crashed run left behind is cleared.

If the socket still cannot be opened, because there is no path to put it at or
another `bdi` is already listening on the one it has, `bdi` says so on stderr
at startup, names the remedy, and polls everything.

## Server and embedded trackers

`bdi` speaks to no database. It asks bd, so what it reads is what bd reads. A
tracker on a Dolt server authenticates, and the password reaches bd in
`BEADS_DOLT_PASSWORD`, from the shell `bdi` was started in or from that
project's `credential_command`. The store `bd init` makes is embedded Dolt,
authenticates to nothing, and needs neither.

The two part company over the cheap question of whether anything moved, which
is a `bd sql` statement the embedded store refuses. `bdi` learns that from the
first refusal of a run and reads such a tracker in full on every poll instead.
That is slower and never wrong, and it shows against a tracker being written
hard: `bdi`'s read waits behind the writes, and a read taking longer than
`unanswered_after_seconds` is reported as a tracker that has stopped answering.

## Each tracker read by its own bd

`bdi` never writes to a tracker, but bd does: on finding itself newer than the
bd that last opened a tracker, bd rewrites `.beads/.local_version` and migrates
the schema, before running whatever subcommand it was given. `--readonly` does
not stop that, and under `--json` bd says nothing about it. So a tracker read
with a bd that is not its project's can be moved to a schema its project's bd
cannot open. That is why `bdi` runs each project's own `bd` through its
environment, and refuses to fall back to its own when that fails. Upgrading a
project's bd migrates on the first read afterwards. `design.md`'s *Reading a tracker is not leaving it alone* has the
measurements.
