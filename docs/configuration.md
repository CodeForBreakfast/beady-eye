# Configuring `bdi`

`bdi` reads `~/.config/beady-eye/config.toml`, or the file `--config` names,
and re-reads it while running: an edit takes effect a couple of seconds later.
A file that does not parse leaves the previous config in force and says so at
the foot until it is fixed.

A key `bdi` does not read is refused, and the refusal names the key and the
table it was written in:

```
unknown field `refresh_second`, expected one of `refresh_seconds`, `unanswered_after_seconds`, `tail_refresh_millis`, `wheel_notch_lines`
in `tui`
```

While running, that is a file that does not parse, and the previous config
stays in force as above.

Everything has a default except the project list:

```toml
[[projects]]
name = "arkham"
path = "/home/you/arkham"

[[projects]]
name = "dunwich"
path = "/srv/work/dunwich"
environment_command = "nix develop -c"
prefix = "dun"

[[projects]]
name = "kadath"
path = "/home/you/dev/kadath"
credential_command = "secret-tool lookup tracker kadath"

[[projects.badges]]
key    = "metadata.delivery_pr"
render = "⇢ kadath/{}"

[roots.explicit]
arkham = ["ark-1", "ark-10"]

[[badges]]
key    = "metadata.delivery_pr"
render = "⇢ {}"

[[badges]]
key    = "metadata.blocked_on"
match  = "human"
render = "⏸ waiting"

[join]
pane_key = "agent_pane"

[changes]
socket = "/run/user/1000/beady-eye/changes.sock"
covered_for_seconds = 60

[watcher]
socket = "/run/user/1000/beady-eye/watcher.sock"

[gates]
poll_seconds = 60
owners = ["dunwich"]

[anomalies]
stale_claim_days = 30

[tui]
refresh_seconds = 30
unanswered_after_seconds = 30
tail_refresh_millis = 250
wheel_notch_lines = 3

[theme]
background = "light"

[tail.crop]
claude = "claude-code"

[row]
identity = ["glyph", "id"]
title    = ["title", "badges"]
state    = ["progress", "agent", "anomalies"]
```

## Which projects a run reads

With a config naming several projects, the directory you start in decides.
Under one of them, meaning its directory, a repository inside it, or a linked
worktree, `bdi` reads that project alone and says so on screen. Anywhere else,
it reads all of them.

`--all-projects` reads every configured project from anywhere. `--project NAME`
(repeatable) reads only those, from anywhere. A bead named on the command line
as `PROJECT:ID` adds its tree to the run, and reads that project if the
directory would have left it out. `bdi` starts focused on that bead, as
`Shift+F` on it would. Under `--json`, a project with a bead named in it
writes only the trees those beads are in.

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

**`prefix`** is the prefix the project's tracker gives its beads, written as
`bd init --prefix` takes it: `prefix = "dun"` for beads named `dun-7`. `bdi`
learns a prefix from the beads a tracker answers with, so this only matters
for a project it has not read. With it, a blocker carrying the prefix is known
to be this project's bead. Where the directory `bdi` was started in chose what
to read, `bdi` then reads this project, draws the blocker as the bead it is,
and goes on reading the project from then on. It draws the blocker's children
and blockers with it, and none of the project's own trees. Under `--project` it says the
blocker is this project's bead, which was not read. A blocker carrying another
prefix is never put down to this project. A project
stating none may hold any blocker whose prefix no answer carries, so the line
lists it among the projects not read.

**`poll = false`** stops polling this project and relies on something
[telling `bdi` when it changed](#telling-bdi-a-project-changed). Nothing then
covers for a producer that dies, so the project's mark turns to `?` once
nothing has vouched for it for `covered_for_seconds`.

**`events_journal = true`** says that every writer to this project's tracker
keeps bd's events journal. [The watcher](#running-the-watcher) then reads
the journal whenever the project has moved, and sends each record to the
consumers watching the bead it names, ahead of the bead's new state. bd turns
the journal on clone by clone, with `bd config set events-journal true`, and
cannot tell a reader whether every other writer has done the same. So the key
is your word for it, and a writer without the journal leaves a gap that
nothing reports. Without the key the watcher sends no records, and tells
each consumer `"events": "off"`.

The watcher reads the whole journal once when it starts, to find where it
ends, and sends none of it. Where bd has pruned the journal past a record the
watcher had not yet read, the watcher tells each consumer once that the
journal is unreadable, and then sends the records bd kept.

## `[roots.explicit]`

Trees to draw beyond the ones `bdi` finds for itself, listed under the project
whose tracker holds them. Bead prefixes are per-tracker, so an id has to be
placed.

## `[[badges]]`

Draw a value beside every bead that carries it. An entry names one value and
says what the row carries for it:

| written | what it says | needed |
|---|---|---|
| `key` | which value of the bead this badge draws | yes |
| `render` | the text the row carries | yes |
| `match` | which values this badge draws on, and how the value comes apart | no |
| `when` | fields of the bead that must each match a pattern for the badge to draw | no |
| `unless` | fields of the bead that stop the badge drawing where any one matches | no |
| `short` | what the row carries instead where `render` will not fit | no |
| `link` | where the badge points | no |
| `colour` | what it is drawn in | no |
| `drawn_on` | which rows it is drawn on | no |

`key` names a value of the bead. A field of the bead is its own name, so
`external_ref` reads the external reference, and whatever `bd` puts on a row is
readable this way, under the name `bd` spells it. A field holding an object is
a value at a time, the two joined with a dot. Metadata is where the names are
yours rather than `bd`'s, and `bdi` has heard of none of them: a key you called
`jira` is read by `metadata.jira`.

The name is split once, so a metadata key of `helio.ticket` is written
`metadata.helio.ticket` and reads as itself.

A badge on `heartbeat_at` or `lease_expires_at` can lag. From beads 1.3.0 a
heartbeat changes only those two values. `bdi` does not re-read a project for
that, so a badge on either of them updates only when something else in the
tracker changes.

A key naming a value the bead does not hold draws no badge and says nothing. So
does one naming a whole object rather than a value inside it, and one naming a
list.

`[join]`'s `pane_key` is a metadata key on its own, with no field in front of
it. A pane id is only ever written in metadata, so there is nowhere else it
could be read from.

`render` is the text, with `{}` for the whole value. `match` restricts the badge
to the values a pattern matches, and the pattern is anchored against the whole
value: `human` draws on `human` and not on `inhumane`.

**Several entries may name one key, and they are tried in the order you wrote
them.** The first whose `match` and conditions read the value is the badge the row draws, and
the ones below it are never tried. So write the shape you expect first and the
shapes you will settle for under it. A value none of them reads draws nothing,
and [What a badge says when it cannot do what you asked](#what-a-badge-says-when-it-cannot-do-what-you-asked)
has that case.

A capture the pattern names is `render`'s to place by that name:

```toml
[[badges]]
key    = "metadata.delivery_pr"
match  = "[^/]+/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ {repo} #{number}"
```

So is any field of the bead, named as `key` names one. Where a capture and a
field share a name, the capture is placed.

`when` and `unless` look at the rest of the bead. Each is a table from a field,
named as `key` names one, to a pattern anchored as `match` is. The badge draws
only where every pattern in `when` matches, and not where any pattern in
`unless` does. A field the bead does not hold reads as empty, so `""` asks for
its absence and `".+"` for its presence. A list, such as `labels`, matches
where any one of its members does. Where you want either of two conditions,
write two badges.

```toml
[[badges]]
key    = "assignee"
when   = { "metadata.agent_pane" = "", labels = "human" }
unless = { status = "closed" }
render = "? {}"
```

`link` is where the badge points, written as a template over the same captures and fields
`render` reads. A badge that has one is drawn underlined, and the underline is
the whole of what a reader sees about it — the URL is nowhere in the text on the
row. The badge is also emitted as a terminal hyperlink, so a terminal that
supports one opens the page when the reader clicks the badge, though some want a
modifier held: [Opening a badge](#opening-a-badge) has that.

```toml
[[badges]]
key    = "metadata.delivery_pr"
match  = "(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ #{number}"
link   = "https://forge.invalid/{owner}/{repo}/pull/{number}"
```

**A `link` naming something this value did not supply is no link at all.** A
`delivery_pr` written as a bare `12` gives an optional `owner` and `repo`
nothing, and a URL built round the parts that were never there points at
somewhere else. The badge still draws its `render`; it just has nowhere to go.

`short` is what the row carries where it has no room for the `render`, written
as a template over those same captures. A badge with only one length is the
first thing a narrow pane drops; one with two survives, saying less:

```toml
[[badges]]
key    = "metadata.delivery_pr"
match  = "(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ {repo} #{number}"
short  = "⇢ #{number}"
```

A shortened badge is one the row kept whole, so it still opens the page.

**A `short` naming something this value did not supply is no short form at
all**, by the rule `link` follows and for the same reason: `⇢ #{number}` with no
number in it is a template on the row where a reference belongs. The badge keeps
its one length, and a pane too narrow for that still drops it.

`colour` is what the badge is drawn in. A badge that names none is drawn in the
tone of the row it sits on.

**`colour = "status"` is the one worth reaching for first.** It draws the badge
in the colour that bead's status is drawn in, which is the colour of its id on
the same row. A ticket in another tracker then reads as red on a blocked bead
and orange on an in-progress one, without your having to say so anywhere. It is
the only name here that follows the bead: every other one draws the same badge
the same way on every row.

```toml
[[badges]]
key    = "metadata.jira"
match  = "(?<ticket>[A-Z]+-[0-9]+)"
render = "{ticket}"
colour = "status"
```

**A slot of `bdi`'s own palette** draws the badge in whatever that slot is drawn
in, and moves with your theme as that slot does. What each slot means is what it
means everywhere else on the screen:

| written | what `bdi` draws in it |
|---|---|
| `agent` | a live agent is here |
| `attention` | this wants looking at |
| `identity` | a bead's id at the head of its window |
| `status_open` `status_in_progress` `status_blocked` `status_closed` `status_deferred` | `bd`'s own colour for each status, as a fixed colour rather than this bead's |
| `tier_staffed` `tier_open` `tier_finished` | the three rungs of how live a row is, as a fixed treatment rather than this row's |
| `structure` | the box-drawing the tree is shaped from |
| `quiet` | metadata, chrome, an affordance, a rule |
| `page` | every row of the bead window |
| `plain` | `bdi`'s own sentence about a forest where nothing went wrong |
| `code` | a code span or a code block |
| `selected` | the row under the cursor |
| `title` | a window's own name, on its border |
| `section` | `bd show`'s section names |
| `heading` `emphasis` `strong` | a heading, an emphasis and a strong word in prose |
| `link` | somewhere to go |

`selected`, `title`, `section`, `heading`, `emphasis`, `strong` and `link` are a
weight rather than a colour, so a badge naming one of them comes out bold,
italic, underlined or reversed and takes the row's tone for its colour. A badge
with a `link` is drawn underlined already, and `colour = "link"` is how to have
the underline without one. `voice`, the tone the tail band speaks in, is the one
treatment `bdi` names that a badge cannot: it is chosen from your declared
background rather than being one colour.

**A colour you write** is drawn exactly as written, and is the one kind your
theme cannot move. Write it as `#rrggbb`, as one of the sixteen by name, or as
an index into your terminal's palette:

```toml
[[badges]]
key    = "metadata.design"
render = "✎ {}"
colour = "#c71585"
```

`bdi` has no idea what your metadata means and draws the badge as written.

`drawn_on` says which rows draw the badge. `"own"`, the default, draws it on its
bead's row alone.

**`drawn_on = "blocked"` also draws it on the row of each bead its bead blocks,**
while that row rests shut and the blocker is still open. A blocker hangs under the
bead it blocks, so the fold keeps the blocker's row off the screen, and what
that row carries, such as a pull request's link, is often why the bead waits.
The badge goes after the row's own, so a bead with two such blockers draws
both. Only a bead blocked directly draws it, and a parent never draws its
child's.

Leave it out for a badge that would read on another row as that bead's own,
such as a ticket key.

```toml
[[badges]]
key      = "metadata.repo"
when     = { await_type = "gh:pr", await_id = "[0-9]+" }
render   = "⇢ #{await_id}"
drawn_on = "blocked"
```

## `[[projects.badges]]`

What one project draws ahead of this list, for the keys it names and no others.
A shared list can only say what the value itself carries, so a project whose
`delivery_pr` leaves out its owner and repository needs an entry of its own to
supply them. Every key a project stays silent about keeps drawing what
`[[badges]]` says.

**A project's entries for a key are tried before the `[[badges]]` entries for
that key, and replace none of them.** They sit where the first `[[badges]]` entry
for the key sat, so naming a key does not move the row's other badges. The shared
entries for that key follow, in the order they were written. Keys only the
project names come last.

So a project wins a value by being tried first, not by taking anything away. A
value its own entries do not read falls through to the `[[badges]]` entries
underneath them, and draws whatever it would have drawn had the project named
nothing.

That is why one entry is usually all a project needs. It names the shape its own
tracker writes, and every other shape keeps being read by the shared list.

A project cannot silence a `[[badges]]` entry. Naming a key puts your own entries
first; it does not take the shared ones away.

**A project's own keys have to come before its badges.** `[[projects.badges]]`
opens a table of its own, so `name`, `path` or anything else written after it
belongs to the badge, which refuses it:

```
unknown field `path`, expected one of `key`, `match`, `when`, `unless`, `render`, `short`, `link`, `colour`, `drawn_on`
in `projects.badges`
```

## Badging the systems you reference

`bdi` draws one reference with no badge configured: the pull request a beads
`gh:pr` gate waits on, which the first shape below describes. Every other
reference is one you write. The other shapes are what a reference usually looks
like, and each is built the same way whichever place it is read from: a `match`
that reads the value apart, a `render` that says what the row carries, and a
`link` that rebuilds the address.

**Which place you read depends on who wrote the reference.** One of beads' sync
adapters fills `external_ref`, whose `tracker.IssueTracker` contract parses and
writes it, so a badge on that field is `BuildExternalRef` run backwards. A
reference you write by hand goes in metadata, under a key naming the tracker it
points at. beads keeps the `bd:` prefix for itself and `_` for its internal
keys, so a metadata key avoids both.

A bead holds one external reference and as many metadata keys as you write, so
a setup referencing several systems badges all but one of them from metadata.

### A pull request a bead waits on

Record the wait as a beads gate on the pull request's number:

```console
$ bd gate create --type=gh:pr --blocks dun-7 --await-id=12
```

The gate hangs under the bead it blocks. Its repository is its `repo`
metadata, written as `OWNER/REPO`, or as `HOST/OWNER/REPO` for a pull request
on a host other than GitHub. `bd gate create` copies `repo` from the bead the
gate blocks. Where that bead has none, set it on the gate:

```console
$ bd update dun-9 --set-metadata repo=dunwich/arkham
```

This badge draws `⇢ arkham #12` on the gate, linked to
`https://github.com/dunwich/arkham/pull/12`. A narrow row draws `⇢ #12`. While
`dun-7` rests shut over the gate, `dun-7`'s own row draws the badge.

```toml
[[badges]]
key      = "metadata.repo"
when     = { await_type = "gh:pr", await_id = "[0-9]+" }
match    = "(?<owner>[A-Za-z0-9_.-]+)/(?<name>[A-Za-z0-9_.-]+)"
render   = "⇢ {name} #{await_id}"
short    = "⇢ #{await_id}"
link     = "https://github.com/{owner}/{name}/pull/{await_id}"
drawn_on = "blocked"
```

A repository on another host takes a second entry under it, which reads the
values the first one does not:

```toml
[[badges]]
key      = "metadata.repo"
when     = { await_type = "gh:pr", await_id = "[0-9]+" }
match    = "(?<host>[A-Za-z0-9_.-]+)/(?<owner>[A-Za-z0-9_.-]+)/(?<name>[A-Za-z0-9_.-]+)"
render   = "⇢ {name} #{await_id}"
short    = "⇢ #{await_id}"
link     = "https://{host}/{owner}/{name}/pull/{await_id}"
drawn_on = "blocked"
```

A gate with no `repo`, or one whose await id is not a number, draws neither.

[`bdi gates`](#settling-pull-request-gates) closes the gate once the pull
request merges, so the bead it blocks becomes ready.

A gate can instead wait for its pull request to leave draft, such as one
holding back a review that starts once the author marks the pull request ready
for review. Write `awaits=ready_for_review` into the gate's metadata:

```console
$ bd update dun-9 --set-metadata awaits=ready_for_review
```

`bdi gates` closes that gate once the pull request is ready for review, or once
it merges.

A gate can also wait for its pull request to be approved, such as one holding
back a step that starts once the team has reviewed it. Write `awaits=approved`
into the gate's metadata:

```console
$ bd update dun-9 --set-metadata awaits=approved
```

`bdi gates` closes that gate once GitHub's review decision on the pull request
is approved, or once it merges. It counts no approving reviews itself.
GitHub gives no review decision on a repository that does not require reviews,
so a gate waiting for approval there waits for the merge.

`ready_for_review` and `approved` are the only values it knows. It reports a
gate with any other `awaits` value and leaves the gate open.

### An issue tracker key

A bead carrying `jira = "HELIO-412"`:

```toml
[[badges]]
key    = "metadata.jira"
match  = "(?<ticket>[A-Z]+-[0-9]+)"
render = "{ticket}"
link   = "https://jira.invalid/browse/{ticket}"
```

The row draws `HELIO-412` underlined, and it opens the ticket.

### A reference a sync adapter wrote

A bead whose `external_ref` holds `https://jira.invalid/browse/HELIO-412`. The
adapter writes a whole URL, so the badge cuts the identifier out of it and
rebuilds the address it already had:

```toml
[[badges]]
key    = "external_ref"
match  = ".*/(?<ticket>[A-Z]+-[0-9]+)"
render = "{ticket}"
link   = "https://jira.invalid/browse/{ticket}"
```

The row draws `HELIO-412` and opens the ticket. The URL is nowhere in the text,
which is what lets the badge sit beside a title.

### A pull request written as an owner, a repository and a number

A bead carrying `delivery_pr = "dunwich/arkham#12"`:

```toml
[[badges]]
key    = "metadata.delivery_pr"
match  = "(?<owner>[^/]+)/(?<repo>[^#]+)#(?<number>[0-9]+)"
render = "⇢ {repo} #{number}"
short  = "⇢ #{number}"
link   = "https://forge.invalid/{owner}/{repo}/pull/{number}"
```

The row draws `⇢ arkham #12` and opens the pull request. The owner never reaches
the row: a capture `render` leaves out is still `link`'s to use. Narrow the pane
and the badge drops the repository rather than the row dropping the badge, and
`⇢ #12` opens the same page.

### A bare number, in a project with only one repository

A bead carrying `delivery_pr = "12"` has nothing in the value to build an
address out of, and a shared list can only say what the value itself carries.
The project's own entry supplies the rest, and is tried before the shared one:

```toml
[[projects]]
name = "kadath"
path = "/home/you/dev/kadath"

[[projects.badges]]
key    = "metadata.delivery_pr"
match  = "(?<number>[0-9]+)"
render = "⇢ #{number}"
link   = "https://forge.invalid/dunwich/kadath/pull/{number}"
```

This is tried first on `kadath`'s beads, and on no other project's, by the rule
[`[[projects.badges]]`](#projectsbadges) gives. The `[[badges]]` entry is still
there underneath it, so a `kadath` bead that does carry an owner and repository
is read by that one as before.

### A reference stored as a full URL

`{}` is the whole value wherever a template takes a capture, `link` included, so
an address needs no rebuilding. `match` still earns its place: it shortens the
row, and it declines a value that is not an address of yours.

```toml
[[badges]]
key    = "metadata.delivery_pr"
match  = "https://forge.invalid/[^/]+/(?<repo>[^/]+)/pull/(?<number>[0-9]+)"
render = "⇢ {repo} #{number}"
link   = "{}"
```

### What a badge says when it cannot do what you asked

`match` is how you say which values you want, so a badge whose pattern does not
read the value draws nothing and reports nothing. The next entry for that key is
tried instead. A value no entry for the key reads stays silent: you said which
shapes you wanted, and none of them was this one. Where you would rather see
every value of a key, write a last entry with a permissive pattern, and it draws
whatever the ones above it declined.

What is worth saying is a badge that read the value and then could not keep one
of the other promises its config made. A `link` was written to point somewhere
and a `short` to survive a narrow pane, and a value that defeats either takes
that away while leaving the badge looking ordinary. So the row says so where it
says its anomalies:

| what the badge met | what the row says |
|---|---|
| a value that left part of the `link` unfilled | `no link for delivery_pr: its value does not fit the link template` |
| a link holding a control character | `no link for delivery_pr: it holds a control character` |
| a value that left part of the `short` unfilled | `no short form for delivery_pr: its value does not fit the template` |
| a `short` holding a control character, on a badge with a `link` | `no short form for delivery_pr: it holds a control character` |

The first `short` row holds whether or not the badge has a `link`. The second is
about the link: a badge is drawn as one at every width or at none, so a short
form the sequence cannot carry is one the row declines rather than a badge that
is underlined at one width and openable at another.

### Opening a badge

`bdi` captures the mouse at startup so the wheel can move the tree, and a
terminal that registers its open-the-link binding for the ungrabbed case only
will never fire it while `bdi` is up. The binding that survives a grab is the
modified one. In kitty a plain click on a badge does nothing and ctrl+shift+click
opens it, and in herdr a plain click opens it. Those two are what has been tried:
where a plain click does nothing in yours, look for the modifier it wants for a
link under mouse capture.

## `[join]`

`pane_key` is the metadata key that names the herdr pane an agent sits in. It
ties an agent to its bead exactly.

## `[changes]`

`socket` is where `bdi` watches for something saying a project's work has
moved. It defaults to `$XDG_RUNTIME_DIR/beady-eye/changes.sock`, and a machine
with no `$XDG_RUNTIME_DIR` has no channel until this names one. `--socket`
overrides it for one run, which is how two `bdi` runs on one machine each get
a channel. [Telling `bdi` where to watch](#telling-bdi-where-to-watch) has
the whole of it.

`covered_for_seconds` is how long a project that does not poll is taken to be
current after its last read, or after the last `covered` line naming it. Past
that its mark turns to `?`. The default of 60 is three of the 20-second
heartbeats a producer reading a Dolt event stream sends, so a late heartbeat
does not read as a producer that has gone. A polled project never lapses.

## `[watcher]`

`socket` is where `bdi watch` takes its socket. It defaults to
`$XDG_RUNTIME_DIR/beady-eye/watcher.sock`, and `bdi watch --socket` overrides
it. The path is checked as `[changes]`'s is. `bdi --json` and `bdi --beads`
make the same check before they connect, and also require the socket to be
yours. Where either fails, they read every project themselves. [Running the
watcher](#running-the-watcher) has the rest.

## `[gates]`

What `bdi gates` settles, and how often.
[Settling pull-request gates](#settling-pull-request-gates) has the rest.

`poll_seconds` is how long `bdi gates` waits after one look at the gates before
the next. The default is 60.

`owners` names the repository owners whose pull requests this `bdi gates`
settles, matched in any case. A gate on a repository of any other owner is left
alone and not reported. Where `owners` is empty, which is the default, every
gate is settled. A gate whose `repo` names no owner is settled only then. A
delivery from GitHub is held to the same rule.

`excluded_owners` names the repository owners this `bdi gates` leaves alone,
matched in any case, whatever `owners` says. Where two instances split the work
by owner, one names its owners in `owners` and the other names the same owners
in `excluded_owners`. Then a gate on any other owner, such as a pull request
upstream in a dependency, is still settled by the second instance. The default
is empty.

The address deliveries are taken on and the secret they are signed with are
not here. They are given on the command line and in the environment, as
[Taking GitHub's deliveries](#taking-githubs-deliveries) says, so the secret
never sits in a config file.

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

## `[tail.crop]`

Where the tail cuts a pane's screen, for each agent. A key is the name herdr
gives the agent in a pane, which `herdr agent list` prints as `agent`. A value
is one of the crops `bdi` ships, and the tail shows the rows above where it
cuts:

| crop | where it cuts |
|---|---|
| `claude-code` | above Claude Code's input box, so the tail shows what the agent last said and when it finished, without the box, the suggestion greyed into it, the meter or the mode line |

```toml
[tail.crop]
claude = "claude-code"
```

A pane whose agent has no entry is tailed uncropped. A screen the crop finds
nowhere to cut, such as one with a dialog drawn where the box would be, is
tailed uncropped too. The file is refused for a crop `bdi` does not ship.

## `[row]`

Which cells a bead's row draws, and in what order. A row is three blocks: the
`identity` at the left, the `title` filling the middle, and the `state` at the
right. Each is a list of cells, drawn in the order written, and the default is
the row as it has always been drawn:

```toml
[row]
identity = ["glyph", "id"]
title    = ["title", "badges"]
state    = ["progress", "agent", "anomalies"]
```

A list left out is the default's. A list written is read as written, so
`title = []` is a row with nothing in its middle block.

| cell | what it draws |
|---|---|
| `glyph` | the bead's status, as `bd list`'s legend draws it |
| `id` | the bead's id, as what it adds to the id above it |
| `title` | the bead's title |
| `badges` | every badge the row does not name on its own, in config order |
| `progress` | the fraction, on a line that stands for more than itself |
| `agent` | the live agent on the bead |
| `anomalies` | what the join found wrong |
| `badge.<key>` | the badges on one `key`, as their `[[badges]]` entries name it |

`badge.<key>` takes those badges out of `badges` and puts them where you wrote
it, so a badge can sit beside the id while the rest stay after the title. A
row holds one badge on a key, and more only where a shut row draws its
blockers' badges after its own, as [`drawn_on`](#badges) says. Adding a badge
to the config needs no edit here: `badges` draws it.

The file is refused for a cell `bdi` cannot draw: a name that is none of the
above, a `badge.<key>` whose key no `[[badges]]` or `[[projects.badges]]`
entry configures, and a cell named twice, in one list or across two.

Notes and the counts on a shut line are not cells: they trail the state
whatever the row says.

## Telling `bdi` a project changed

`bdi` polls, and most polls find nothing moved. A poll first asks the tracker
whether anything has changed (one `bd sql` for a hash of its Dolt tables) and only
reads in full if it has. That probe needs a Dolt server; bd's embedded store
refuses it, and `bdi` then reads in full on every poll.

Anything that already knows a tracker changed can skip the wait. `bdi` watches
on a stream socket, created mode `0600` and removed on exit,
`$XDG_RUNTIME_DIR/beady-eye/changes.sock` unless it is told otherwise. Write a
project's name as one line; `bdi` reads that project now and answers on the
same connection:

| answer | meaning |
|---|---|
| `ok <project>` | read again now |
| `unknown <project>` | not a project this run is reading |
| `malformed` | blank, or over 512 bytes |

A producer watching a project that nothing has changed in writes
`covered <project>` instead. It is answered `ok <project>` too, and nothing is
read. It is how a producer that speaks only on change says it is still there,
which matters most for a project with `poll = false`: that project's mark turns
to `?` once nothing has vouched for it for `covered_for_seconds`. A `bdi`
older than the word answers `unknown covered <project>`.

A connection can carry as many lines as you like and stay open for as long as
the writer does. A project that is reported for is never polled, since each
report pushes the next poll past its interval, and so does each `covered`
line. One whose producer goes quiet is polled again from one interval later.

The cheapest producer is a wrapper round `bd` itself. It reads the default
path; a `bdi` told a different one has to be told to the producer too. It
writes with socat, or with OpenBSD netcat where socat is absent, and says so
when it has neither rather than lose the report.

```bash
bdi_changed() {
  local sock="$XDG_RUNTIME_DIR/beady-eye/changes.sock"
  [ -S "$sock" ] || return 0
  if command -v socat >/dev/null; then
    printf '%s\n' "$1" | socat - UNIX-CONNECT:"$sock" >/dev/null 2>&1
  elif command -v nc >/dev/null; then
    printf '%s\n' "$1" | nc -N -U "$sock" >/dev/null 2>&1
  else
    echo "bdi_changed: no socat or nc, so $1 was not reported" >&2
    return 1
  fi
}

bd() {
  command bd "$@" || return
  case "$1" in
    create|update|close|note|dep) bdi_changed my-project ;;
  esac
}
```

A Dolt trigger, a git hook, a systemd path unit or a cron job comparing a head
hash all work equally well; `bdi` provides the socket and cannot tell them apart.

`--poll` and `--no-poll` override every project's `poll` setting for one run,
which is how to find out whether a suspect producer was the only thing wrong.

### Telling `bdi` where to watch

The default path is one per login session, so two `bdi` runs on one machine
derive the same one and the second finds the first already watching. It says
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
another `bdi` is already watching the one it has, `bdi` says so on stderr
at startup, names the remedy, and polls everything.

## Running the watcher

`bdi watch` is a `bdi` with no view. It reads every project the config names,
polls and probes each one as a view does, and holds each bead as bd printed it,
with whether it is ready and what blocks it. Its config is read once at
startup, so an edit to the config takes effect at the next start. It runs
nothing of herdr's.

It has a socket of its own, apart from the one a view watches, and
takes the same producer lines: a project's name, or `covered <project>`. A
producer that should reach the watcher is pointed at
`$XDG_RUNTIME_DIR/beady-eye/watcher.sock`, or at the path `[watcher]` names.
A consumer watching beads connects to the same socket. The README has a worked
one. The view, `bdi --json` and `bdi --beads` find the watcher at the same
path and read through it, so a `[watcher]` socket is named once for all
three. They read a project
themselves where the watcher's config gives it another `path` or
`environment_command` than theirs does.

Run one per machine. A second `bdi watch` finds the first by connecting to the
socket, says on stderr which socket is taken, and exits non-zero. One that
cannot open its socket for any other reason exits the same way.

Start it under whatever supervises your processes. A systemd user unit, at `~/.config/systemd/user/bdi-watch.service`:

```ini
[Unit]
Description=beady-eye watcher

[Service]
ExecStart=%h/.cargo/bin/bdi watch
Restart=on-failure

[Install]
WantedBy=default.target
```

```console
$ systemctl --user enable --now bdi-watch
```

The watcher reaches each tracker with the environment it starts in, so a
tracker whose credential comes from your shell wants a `credential_command` in
its `[[projects]]` entry, or an `Environment=` line here.

A launchd agent on macOS, at `~/Library/LaunchAgents/com.example.bdi-watch.plist`.
macOS has no `$XDG_RUNTIME_DIR`, so the socket is named:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>com.example.bdi-watch</string>
  <key>ProgramArguments</key>
  <array>
    <string>/Users/you/.cargo/bin/bdi</string>
    <string>watch</string>
    <string>--socket</string>
    <string>/Users/you/Library/Caches/beady-eye/watcher.sock</string>
  </array>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
```

```console
$ launchctl load ~/Library/LaunchAgents/com.example.bdi-watch.plist
```

SIGTERM, SIGINT or SIGHUP stops it and removes its socket.

## Settling pull-request gates

`bdi gates` settles every configured project's gh:pr gates. It looks at every
open gh:pr gate, asks GitHub once about each pull request one of them waits
on, and acts on what GitHub says:

| the pull request | what `bdi gates` does |
|---|---|
| merged | closes each gate waiting on it with `bd gate resolve`, naming the merge commit, so the beads it blocked become ready |
| closed without being merged | leaves each gate open, and comments once on each bead a gate holds back |
| open and ready for review | closes each gate waiting on it with `awaits=ready_for_review` in its metadata, and leaves the rest open |
| open and approved | closes each gate waiting on it with `awaits=approved` in its metadata, and leaves the rest open |
| open or a draft, and its head commit has failing checks | leaves each gate open, and comments once on each bead a gate holds back for that commit, so a fix that fails again is told again |
| open or a draft, and reviewed | leaves each gate open, and comments on each bead a gate holds back once for each review submitted, approving, requesting changes or commenting, naming the reviewer and the state. A dismissed review is not told. It reads the latest five reviews |
| open or a draft, and GitHub says its head commit conflicts with its base | leaves each gate open, and comments once on each bead a gate holds back for that commit, so a rebase that still conflicts is told again. A pull request GitHub has not yet worked out is not told |
| open or a draft, and commented on | leaves each gate open, and comments on each bead a gate holds back once for each comment on the pull request's conversation, naming the comment's author and linking it. Comments from a review are not told here. There is no author filter, so a seat's own comments arrive too. It reads the latest five comments |
| a draft with no failing checks, no review, no conflict and no comment | nothing |

A gate tells only what happened after it was made, since whoever made it could
already see the rest. A review dates from when it was submitted, a comment from
when it was made, and failing checks from when the first check to fail finished.
GitHub does not say when a conflict began, so a conflict dates from the later of
the head commit and the base branch's latest commit. A conflict that stood
before the gate is still told once the base moves on. A close without a merge
is told however old the gate is.

It never creates a gate. Whoever opens the pull request creates one with `bd`,
as [A pull request a bead waits on](#a-pull-request-a-bead-waits-on) shows.
Then it waits [`[gates] poll_seconds`](#gates) and looks again, until it is
stopped. Its config is read once at startup.

It asks GitHub through `gh`, so it settles what the account `gh` is signed in
to can see. A look asks about a repository's pull requests together, up to a
hundred in one query, so what it spends of that account's rate limit grows with
the repositories rather than the pull requests. To settle repositories that
need different accounts, run one `bdi gates` per account, each with its own
config naming its [`[gates] owners`](#gates), or the owners it leaves to the
others in `excluded_owners`.

Each look reports on stdout one line for each gate closed, each bead told, and
each failure:

```
dunwich/arkham#12 merged: arkham closed gate ark-0i5
dunwich/arkham#7 closed unmerged: arkham told ark-2ud
dunwich/arkham#15 is ready for review: arkham closed gate ark-eb1
dunwich/arkham#18 has failing checks: arkham told ark-7mw
dunwich/arkham#18 was reviewed: arkham told ark-7mw
dunwich/arkham#22 conflicts with its base: arkham told ark-3xq
dunwich/arkham#18 was commented on: arkham told ark-7mw
kadath: its gh:pr gates could not be read: the tracker did not answer
dunwich/arkham#30: GitHub did not say where it stands, so no gate waiting on it was touched: gh exited 1 for a reason bdi cannot place
dunwich/arkham#18: bdi gates cannot see whether it has failing checks, so it acts on everything else and says this once for dunwich/arkham: GitHub would not let gh read commits: Resource not accessible by personal access token
arkham: gate ark-6pp cannot be settled: it names no repo
dunwich/arkham#41: GitHub refused it for the rate limit of the login gh runs as, so GitHub is asked nothing more until the limit resets at 2026-01-01 00:30:00 UTC
```

A failure stops nothing. The next look tries again. A `gh` that GitHub refuses
leaves every gate waiting on that pull request as it was. On an organisation
that enforces single sign-on, a lapsed authorisation is the usual cause, and
`gh auth refresh` is the cure.

A field GitHub will not show the account `gh` runs as costs only the events
that read it. Every other event still settles the pull request, merges
included, and `bdi gates` names the event it cannot see once for each
repository, with what GitHub said.

A rate limit is the exception, because the login `gh` runs as may be shared
with whoever else uses it. Once GitHub refuses a pull request for one, that
look asks nothing more, and `gh api rate_limit`, which costs nothing, says when
the limit resets. The next look waits until then. A secondary limit's end is not
said, so the next look waits at least a minute. A delivery during the wait is
not settled, and the next look settles it.

A systemd user unit, at `~/.config/systemd/user/bdi-gates.service`:

```ini
[Unit]
Description=beady-eye gate settling

[Service]
ExecStart=%h/.cargo/bin/bdi gates
Restart=on-failure

[Install]
WantedBy=default.target
```

```console
$ systemctl --user enable --now bdi-gates
```

As with the watcher, each tracker is reached with the environment the unit
starts in, and so is `gh`.

### Taking GitHub's deliveries

A repository that can send GitHub webhooks need not wait for the next look.
Given `--listen`, `bdi gates` also takes deliveries over HTTP on that address,
and settles the pull request each one names as it arrives:

```console
$ BDI_GATES_WEBHOOK_SECRET=… bdi gates --listen 0.0.0.0:8080
```

| setting | what it is |
|---|---|
| `--listen <address>` | the address and port to take deliveries on, such as `0.0.0.0:8080`. Port `0` takes one the system picks, and the line `bdi gates` starts with names it |
| `--webhook-secret-file <path>` | a file holding the secret the webhook was given on GitHub |
| `BDI_GATES_WEBHOOK_SECRET` | the secret, where no file is named. `bdi gates` takes it out of its environment once read, so no `bd` or `gh` it starts is handed it |

Whitespace around the secret is dropped, and `--listen` with no secret, or an
empty one, refuses to start.

On GitHub, give the webhook the address `bdi gates` is reached at, the content
type `application/json`, the same secret, and the *Pull requests*, *Pull request
reviews*, *Issue comments*, *Check suites* and *Statuses* events. A delivery is
a trigger only. `bdi gates` reads the repository and number out of it and settles that pull
request exactly as a look would, asking GitHub where it stands, and passes it
over without a word where [`[gates]`](#gates) leaves its owner to another
`bdi gates`. A check suite or status names a commit and no pull request, so
`bdi gates` asks GitHub for the open pull requests whose head is that commit
and settles each. Each delivery is answered before it is settled:

| the request | the answer |
|---|---|
| a `pull_request`, `pull_request_review`, `issue_comment` on a pull request, `check_suite` or `status` delivery signed with the secret | `202`, then the pull requests it names or whose head it names are settled |
| an `issue_comment` delivery on an issue that is not a pull request, or any other event, GitHub's `ping` among them, signed with the secret | `202`, and nothing else |
| a delivery with no `X-Hub-Signature-256`, or one the secret did not make | `401`, and a line on stdout |
| a signed `pull_request`, `pull_request_review`, `issue_comment`, `check_suite` or `status` delivery naming no repository and pull request or commit | `400`, and a line on stdout |
| a body over 1 MiB, far more than any `pull_request` delivery | `413`, and a line on stdout |
| a delivery with no `Content-Length`, such as a chunked one | `411`, and a line on stdout |
| any request while eight are already being answered, or a signed delivery to settle while 64 wait to be settled | `503`. GitHub does not send it again, so the next look settles it |
| a request that has not arrived in full within ten seconds | the connection is closed unanswered |
| `GET /healthz` | `200`, for a readiness probe, or `503` while GitHub refuses every read it is asked |

The probe fails once a look or a delivery has asked GitHub about pull requests
and been refused every time, whether the login's token has expired or been
revoked, or GitHub is holding it to a rate limit. It passes again once GitHub
answers a read. A look that asks GitHub nothing, because no gate waits on a
pull request, leaves the answer as it was.

Deliveries and looks are settled one at a time, on one thread, so a delivery
arriving during a look waits for it, and the two never act on one gate
together. Looks go on at `[gates] poll_seconds` whatever arrives, which is what
settles a pull request whose delivery was lost.

`bdi gates` speaks plain HTTP. Where it is reached from the internet, put it
behind something that terminates TLS.

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

`bdi` writes to a tracker only when `bdi bd` records a person's answer, but a
bd older than 1.3.0 also writes when it is asked to read: on finding
itself newer than the bd that last opened a tracker, it rewrites
`.beads/.local_version` and migrates the schema, before running whatever
subcommand it was given. `--readonly` does not stop that, and under `--json` bd
says nothing about it. bd 1.3.0, the version the flake pins, does neither under
`--readonly`: it leaves the file alone, and refuses a store whose schema is
behind it until a bd run without the flag migrates it.

So a tracker read with a bd that is not its project's can still be moved to a
schema its project's bd cannot open, wherever that bd predates 1.3.0. That is
why `bdi` runs each project's own `bd` through its environment, and refuses to
fall back to its own when that fails. Upgrading a project's bd to another
version older than 1.3.0 migrates its tracker on the first read afterwards.
Upgrading it to 1.3.0 fails every read until something else migrates the store.
`design.md`'s *Reading a tracker is not leaving it alone* has the measurements.
