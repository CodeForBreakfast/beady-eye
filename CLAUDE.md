# beady-eye — working notes

`bdi` joins a beads tracker to a herdr session and draws one tree of work per
root, annotated with the live agent on each node. Read-only.

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
`tests/fixtures/` are faithful captures of what `bd list`, `bd dep tree`, `bd
query` and `herdr agent list` put on the wire.

Two things to know before writing a test:

A test about colour asks `painted()` in `view/draw/`. Its neighbour `drawn()`
reads `symbol()` only and is blind to styling, which is how a colour bug
shipped. `tui/` has a `painted()` of its own that is no better: it is named
for the `paint()` it calls, returns symbols, and sees no colour either.

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

## PR policy

Changes reach `main` through a pull request, squash-merged — nothing is pushed
to `main` directly. The pull request is what puts CI in front of a change
before the branch everyone else works from carries it.

## Where things are

`src/` is four layers:

- `collect/` runs `bd` and `herdr` and parses what they say.
- `model/` joins the two into a snapshot: the tree, its badges, and the
  anomalies where the two sources disagree.
- `view/` turns a snapshot into rows and draws them — `forest/` is the
  scrollable tree, `draw/` the widgets, `phrase.rs` every word `bdi` shows.
- `tui/` and `app.rs` run the loop and own the terminal.

`docs/design.md` is the spec, and a starting point rather than gospel — we
deviate from it as we learn. `docs/plans/` holds the implementation plans
written against it.

## Rules this project was designed under

**Terminology comes from beads or herdr.** Never invent a word where either
project has the concept. Where both are silent, coin one and add it to the
terminology table in `docs/design.md` marked *coined*.

**No coupling to any agent workflow.** `bdi` knows nothing about how agents are
organised — no roles, no orchestration model, no skill names. A convention a
setup encodes in bead metadata is named in config (`[roots]`, `[[badges]]`,
`join.pane_key`) and drawn without interpretation. If a feature needs to know
what a metadata key *means*, it belongs in config, not in the model.

**The key is `(project, id)`.** Bead prefixes are per-tracker and uncoordinated.

**Degrade, never disappear.** An unreachable tracker, a filtered tree, a dangling
parent, a truncated subtree — each is reported, never silently dropped.

## The tracker

The maintainers track work in a [bd (beads)](https://github.com/gastownhall/beads)
tracker that is not part of this repository — external contributors don't need
it and should use GitHub issues instead. `.beads/` is gitignored, and nothing
tracked here names the tracker, its server or its credentials. What a maintainer
needs to reach it lives in `CLAUDE.local.md`, untracked alongside them.
