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

`nix flake check` is the whole of CI, and it is what to run before you push.
Everything CI runs comes from the flake's `checks` output — the build and tests,
`clippy -D warnings`, `cargo fmt --check`, and a `cargo package` verify — so a
check added there is a check CI runs, and nothing runs that is not there.

Commit before you run it. Untracked files are invisible to it, so a green check
on a dirty tree has not compiled your new files, and `Git tree is dirty` is the
only warning you get.

The check needs `bd`, and not as a tracker client: `tests/no_config.rs` runs
`bdi` the way a fresh machine would, and `bdi` asks `bd` where the tracker is.
Telling "bd is here and nothing tracks this directory" apart from "bd is not
installed" is what that test exists to check, so the binary has to be present.
The flake supplies it.

Unit tests live in `#[cfg(test)]` modules inside the file they cover; `tests/`
holds the integration tests, which run the built binary. Fixtures under
`tests/fixtures/` are faithful captures of what `bd list`, `bd dep tree`, `bd
query` and `herdr agent list` put on the wire.

Two things to know before writing a test:

A test about colour asks `painted()` in `view/draw.rs`. Its neighbour `drawn()`
reads `symbol()` only and is blind to styling, which is how a colour bug
shipped. `tui/` has a `painted()` of its own that is no better: it is named
for the `paint()` it calls, returns symbols, and sees no colour either.

A path with no non-test caller is described by its tests, not covered by them.
The suite goes green, the reader sees green, and the product does something
else — and nothing here warns you, because every module is `pub` and that
switches Rust's own dead-code lint off across the crate.

## Where things are

`src/` is four layers:

- `collect/` runs `bd` and `herdr` and parses what they say.
- `model/` joins the two into a snapshot: the tree, its badges, and the
  anomalies where the two sources disagree.
- `view/` turns a snapshot into rows and draws them — `forest.rs` is the
  scrollable tree, `draw.rs` the widgets, `phrase.rs` every word `bdi` shows.
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

## Tracker and packaging

The maintainers track work in a [bd (beads)](https://github.com/gastownhall/beads)
tracker that is not part of this repository — external contributors don't need
it and should use GitHub issues instead. `.beads/` is gitignored, and nothing
tracked here names the tracker, its server or its credentials. What a maintainer
needs to reach it lives in `CLAUDE.local.md`, untracked alongside them.
