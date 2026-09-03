# beady-eye — working notes

`bdi` joins a beads tracker to a herdr session and draws one tree of work per
root, annotated with the live agent on each node. Every bd command line it
spells is a read, and `collect/` spells all of them.

`README.md` is the outside view — what `bdi` is for, how to install it, and the
socket it listens on. This file is for working on it.

## Building and testing

`nix develop` gives you the toolchain: `cargo`, `clippy`, `rustfmt`,
`rust-analyzer`, and `rerun-bdi-on-change`, which starts `bdi` and puts it back
whenever `src/` changes, so a copy left running in a terminal stays current
without being quit and started again by hand. `cargo build` and `cargo test`
work as usual inside the shell.

`nix flake check` is the whole of CI, and `check-before-push` is what to run
before you push: it runs the check, and refuses a dirty tree rather than check a
source nix cannot see all of. Everything CI runs comes from the flake's `checks`
output — the build and tests, `clippy -D warnings`, `cargo fmt --check`, and a
`cargo package` verify — so a check added there is a check CI runs, and nothing
runs that is not there.

The dependency graph is compiled in a derivation of its own, keyed on
`Cargo.lock` rather than on the source, and every check that compiles unpacks it
before it starts. So an edit under `src/` costs you this crate and nothing else,
and a change to the lock costs you the graph. There are two of those
derivations, because a check reuses one only at the cargo profile it was built
at: release for the build and tests, dev for everything else. Give a check a
command at a profile its artifacts were not built at and cargo compiles the
graph again without saying so — `checkOf` reads each build back and fails on
that rather than pass slowly.

A change is checked twice, once on its pull request and once on the squash, and
the second run pays for it only when the tree is new. CI seeds each check's
output into the shared cache, so a squash carrying a tree its pull request
already checked substitutes every one of them: `nix flake check` prints
`running 0 flake checks` and the job finishes in about thirty seconds instead
of five minutes. That is nix saying the inputs are identical rather than a
check being skipped — a tree no pull request saw, main having moved under a
branch between its verdict and its squash, hashes differently and still gets a
real build. What it costs you is that re-running a green job cannot force a
real check of that tree: nothing about the tree has changed, so it substitutes
again.

Once it is pushed, `read-ci-verdict [<commit>]` says whether CI passed for it,
and `read-ci-verdict --help` says why an empty answer from `gh` is not one.

The check needs `bd`, and not as a tracker client: `tests/no_config.rs` runs
`bdi` the way a fresh machine would, and `bdi` asks `bd` where the tracker is.
Telling "bd is here and nothing tracks this directory" apart from "bd is not
installed" is what that test exists to check, so the binary has to be present.
The flake supplies it.

Unit tests live in `#[cfg(test)]` modules inside the file they cover; `tests/`
holds the integration tests, which either link the library or run the built
binary. A test that reaches a module no test has reached before needs that
module opening up in `src/lib.rs`, which says why. Fixtures under
`tests/fixtures/` are faithful captures of what `bd list`, `bd query` and
`herdr agent list` put on the wire.

Four things to know before writing a test:

A pty test drives `bdi` through `tests/terminal/driver.rs`, which drains the
terminal on a thread of its own from the moment `bdi` starts. That is not a
convenience. A terminal a person is sitting at empties itself whatever the
person is doing, and a harness that reads only when a test asks puts `bdi`
under backpressure no terminal applies: the tty's output queue fills, the next
write blocks, and everything `bdi` would have done after drawing does not
happen. The queue's size is the platform's, so the same test passes on one
machine and hangs on another with nothing in the code to say why — on macOS it
is small enough to fill during the first frame, which cost `bdi-54w.2` an
evening reading a ten-second wait for a collection as a defect in `bd`. So
never read the master yourself, and never take a test's silence as a licence to
stop draining.

An absence assertion has to name the thing whose absence it means, and on this
screen that is rarely a bead's id. The bead window is drawn from the
*selection* rather than from the bead it was opened on, so a screen that
wrongly kept a window up after a collection draws it over whatever the
selection landed on and under that bead's name — and a test looking for the
opened bead's title to be gone finds it gone. It executes the line it is about,
cannot observe it, and passes. What says a window is up is the words every
window says (`Esc to go back`), and the same question is worth asking of any
`!contains` on this screen: could the thing you named have moved rather than
gone?

A test that walks the selection over the screen calls `walk::until` in
`src/view/walk.rs`, which presses inside a count taken before the walk starts
and says which row it never reached. The `screen-walks` check refuses the other
kind, so writing one costs you a CI round trip rather than a wrong tally.

An item nothing calls is a compiler warning here: `src/lib.rs` keeps the
library's modules private, so `dead_code` sees the whole crate and the
`dead-code` check fails on one. It will not see a live item on a path
production never takes — `dead_code` is item-level reachability and does no
value-flow analysis, so a function reached only through an arm of a caller that
short-circuits before it is invisible to the compiler. That one is still
described by its tests rather than covered by them: the suite goes green, the
reader sees green, and the product does something else. It surfaces when
somebody reads the caller for another reason, which is not something you can
schedule — so when you change a caller, check what the arms below its early
return are still reached by.

Three things a mutation run will meet here, so the tally is read as what it is:

`impl View for Screen` in `src/tui/` is reached by the pty harness under
`tests/terminal/` and by nothing else, so a survivor there is a path no pty
test takes rather than a method no test can reach. There is no negotiation to
do: a pty test sends a mouse report by writing xterm's SGR encoding at `bdi`
the way it writes a key, and crossterm parses it whether or not the terminal
ever answered the capture request. Measured 2026-09-03 over the impl's own 21
mutants: 20 caught, and the one that survives is
`rereads_in -> Some(Duration::default())`.

That one is equivalent rather than uncaught, and it was measured rather than
argued: `sleeps_for` returns zero, so the loop stops sleeping and spins, but
`Shown::reread` still gates on its own deadline and the band asked for its
pane 38 times over the window it asks 34 times over unmutated. Same bytes, same
instants, and a core.

The other one — `rereads_in -> None` — is the cautionary tale, because it is
catchable and the obvious instrument cannot catch it. **The band's clock is
never the only one running:** a project that has been read ages in seconds,
that age is a deadline, and the loop reads the pane on every wake whichever
deadline woke it. So a `bdi` that has forgotten the band's clock entirely
still shows what changed, once a second instead of four times a second, and a
test that swaps what a pane says and waits for the new words passes either
way. Timing that one answer does separate them here — ten runs each, 250–252ms
against 606–855ms — but the second figure is a second minus however long `bdi`
took to start, so the two bands meet on a machine slower than this one.
**Counting is what has no phase in it**: over a two-second window at a 50ms
interval the band asks 34 times with its clock and 2 without, and 2 is the
ceiling the ages impose on every machine.

`Shown`, in the same file, has no survivor at all: the unit tests around it
catch every viable mutant in it, which is why the adapter has little of its
own left to miss. Keep it that way. `clicked` used to work out the forest's
geometry itself instead of delegating, and it was the one method of the impl
whose mutants a pty test could not reach for a reason that was not the
harness's — a mutant surviving in an adapter that holds behaviour of its own
says the behaviour is in the wrong layer, not that a test is missing. Moving
the geometry to `Shown::clicked` is what closed it.

A `Timeout` is a third mutation answer and the tally cannot say which kind it
is. Some are genuinely non-terminating in production and not gaps: the three
mutants that feed `fold_all`'s `while self.point_every_drawn_fold(true) {}`
are that category, and so is `+` to `*` on the step past a bare `ESC` in
`sgr::line` (`src/view/sgr.rs`): `ESC` is one byte, so `at * 1` leaves the
row where it was and the next search finds the same `ESC` for ever. Read a
`Timeout` under `src/view/` against that list before calling it a hole. A runaway need not time out at all: `delete !` in
`beneath` (`src/model/tree.rs`) pushes every way back up the tree for ever, so
the test process grows to whatever memory cap the run is under in seconds and
is killed there, which scores it caught. Run cargo-mutants under a cap that
kills the one process and lets the run carry on, or that kill is the end of
the run.

An `unviable` is a fourth answer, and the best hidden of the four, because a
run carrying them reads as a clean pass. cargo-mutants replaces a function's
body with a value of its return type, and where the only value it can reach is
`Default::default()` on a type that has no `Default`, the mutant does not
compile: it is scored unviable and the function is never tested at all. Nothing
in the summary line says so, because the viable mutants elsewhere in the run
were all caught. Measured on the change that landed as `068213c` (#75), whose
whole subject was `view::tail`'s three-state match — 144 mutants, 90 caught, 54
unviable, zero missed and zero timeout, with `src/view/tail.rs` contributing
exactly two of them, one site listed twice, both `replace tail -> Tail with
Default::default()` and both unviable because `Tail` has no `Default`. The
match the change existed to build was scored by nothing while the tally read
zero missed.

So read this one off the file list rather than the summary line, and read it as
a subtraction rather than a tally: a file with unviable mutants is ordinary,
and a file with *only* unviable mutants is the hole. Strip the line and column
from the paths in `mutants.out`'s `unviable.txt`, subtract the paths in
`caught.txt`, `missed.txt` and `timeout.txt`, and what survives is every file
the run scored nothing in.

It is the vacuous run arriving from the other direction, which is why a reader
who knows that one will not expect this one. A diff with no mutable production
line — documentation, or tests alone — makes cargo-mutants print *No mutants to
filter* and exit 0, the same 0 a run that caught everything exits: it scores
nothing and says so, and the count is what tells you. Here it scores plenty,
and the nothing is confined to the file you came to ask about; the file list is
what tells you, and no count carries it.

## PR policy

Changes reach `main` through a pull request, squash-merged — nothing is pushed
to `main` directly. The pull request is what puts CI in front of a change
before the branch everyone else works from carries it.

Read the **file list** before you merge, as a check of its own rather than as
part of reading the diff: `git diff --stat origin/main HEAD` lists every file
the branch touches. Take it once you have merged `origin/main` into the branch
— taken before that merge it also lists what `main` gained meanwhile, in
reverse, and the list is then not your branch's.

Nothing else here asks that question. Every other gate reads content — the
build, the tests, `clippy`, `cargo fmt`, a grep over the diff — and a content
check finds a bad line in a file that belongs. It cannot find a file that
should not be there at all, because every line of such a file reads as exactly
what it is and none of it breaks a build. `check-before-push` does not close
it either: what it refuses is a dirty tree, and a stray file that has been
committed leaves the tree clean. That is how `f8395eb` (#30) put a kept
`mutants.out.old.first/` on `main` on 2026-09-02 — 155 files and 84,462 lines
beside the twelve the change meant to touch, with every gate green. Widening
`.gitignore` closed that path; the file list is what closes the next one.

## Where things are

`src/` is five layers:

- `collect/` runs `bd` and `herdr` and parses what they say.
- `app/` decides what each tracker is asked for and keeps what came back
  between asks — `tracker.rs` is one project's read, `collection.rs` the
  standing set of them.
- `model/` joins the two into a snapshot: the tree, its badges, and the
  anomalies where the two sources disagree.
- `view/` turns a snapshot into rows and draws them — `forest/` is the
  scrollable tree, `draw/` the widgets, `phrase.rs` every word `bdi` shows.
- `tui/` runs the loop and owns the terminal.

`docs/design.md` is the spec, and a starting point rather than gospel — we
deviate from it as we learn. `docs/plans/` holds the implementation plans
written against it.

## Rules this project was designed under

**Terminology comes from beads or herdr.** Never invent a word where either
project has the concept. Where both are silent, coin one and add it to the
terminology table in `docs/design.md` marked *coined*.

**No coupling to any agent workflow.** `bdi` knows nothing about how agents are
organised — no roles, no orchestration model, no skill names. A convention a
setup encodes in bead metadata is named in config (`[[badges]]`,
`join.pane_key`) and drawn without interpretation. If a feature needs to know
what a metadata key *means*, it belongs in config, not in the model.

**The key is `(project, id)`.** Bead prefixes are per-tracker and uncoordinated.

**Degrade, never disappear.** An unreachable tracker, a filtered tree, a
dangling parent — each is reported, never silently dropped.

## The tracker

The maintainers track work in a [bd (beads)](https://github.com/gastownhall/beads)
tracker that is not part of this repository — external contributors don't need
it and should use GitHub issues instead. `.beads/` is gitignored, and nothing
tracked here names the tracker, its server or its credentials. What a maintainer
needs to reach it lives in `CLAUDE.local.md`, untracked alongside them.
