# beady-eye — working notes

`bdi` joins a beads tracker to a herdr session and draws one tree of work per
root, annotated with the live agent on each node. Every bd command line it
spells is a read, and `collect/` spells all of them. `docs/design.md`'s
*Reading a tracker is not leaving it alone* has the measurements.

## Building and testing

`nix develop` gives you the toolchain: `cargo`, `clippy`, `rustfmt` and
`rust-analyzer`. `cargo build` and `cargo test` work as usual inside the shell.

`nix flake check` is the whole of CI, and `check-before-push` is what to run
before you push: it runs the check, and refuses a dirty tree rather than check a
source nix cannot see all of.

Once it is pushed, `read-ci-verdict [<commit>]` says whether CI passed for it,
and `read-ci-verdict --help` says why an empty answer from `gh` is not one.

Unit tests live in `#[cfg(test)]` modules inside the file they cover; `tests/`
holds the integration tests, which either link the library or run the built
binary. A test that reaches a module no test has reached before needs that
module opening up in `src/lib.rs`, which says why. Fixtures under
`tests/fixtures/` are faithful captures of what `bd list`, `bd query` and
`herdr agent list` put on the wire.

A pty test drives `bdi` through `tests/terminal/driver.rs`, which drains the
terminal on a thread of its own from the moment `bdi` starts. So never read the
master yourself, and never take a test's silence as a licence to stop draining.

Stopping is a deadlock rather than a delay. A `bdi` blocked in `write` cannot
finish exiting, and a process that cannot finish exiting is never reaped — so a
wait for it that stops reading in order to wait is each side waiting for the
other, and it does not end.

An absence assertion has to name the thing whose absence it means, and on this
screen that is rarely a bead's id. The bead window is drawn from the
*selection* rather than from the bead it was opened on, so a screen that
wrongly kept a window up after a collection draws it over whatever the
selection landed on and under that bead's name — and a test looking for the
opened bead's title to be gone finds it gone. It executes the line it is about,
cannot observe it, and passes. What says a window is up is the words every
window says (`Esc to go back`).

## PR policy

Changes reach `main` through a pull request, squash-merged — nothing is pushed
to `main` directly. The pull request is what puts CI in front of a change
before the branch everyone else works from carries it.

### The subject

The pull request's **title** becomes the commit subject, because the merge is a
squash — so it is the only line of the branch `main` keeps. The branch's own
commit messages are squashed away, but they stay reachable on GitHub after the
merge, so they are read as well. Write the title as a conventional commit:

    type(scope): description

- **type** is one of `build`, `chore`, `ci`, `docs`, `feat`, `fix`, `perf`,
  `refactor`, `revert`, `style`, `test`. A `!` before the colon marks a breaking
  change.
- **scope** is optional and closed: `collect`, `app`, `model`, `view`, `tui` —
  the five layers under *Where things are* — plus `ci`, `flake`, `docs`,
  `tests`, `deps`. Leave it out rather than coin one; a new scope is a change to
  the check.
- **description** is lower case, in the imperative, and has no full stop at the
  end. It has to finish the sentence *"If applied, this commit will …"* — so
  `draw a bead id in its status colour`, never `draws`, `drew` or `drawing`. An
  acronym or a name keeps its capitals: `GitHub`, `CI`, `NO_COLOR`.
- **72 characters**.

Say what changed in the subject and why in the body, in a sentence or two. A
subject carrying the reason wraps in `git log --oneline`, in blame and in
bisect — three places a reader meets it and none where they want the argument.

The `conventional subject` job refuses a title that is none of this. Run
`conventional-subject '<title>'` in the dev shell for the same verdict before
you open the pull request.

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
