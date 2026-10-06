# beady-eye — working notes

`bdi` joins a beads tracker to a herdr session and draws one tree of work per
root, annotated with the live agent on each node. The eye only looks, and
each thing it does in the world is a pseudopod, a limb grown for one job. It
has two: `bdi bd` passes `bd human respond` through to carry a person's answer
to a bead, and `bdi gates` settles a gh:pr gate once GitHub says its pull
request has merged or closed. Every other bd command line `bdi` spells is a
read, and `collect/` spells all of them.
`docs/design.md`'s *Reading a tracker is not leaving it alone* has the
measurements.

## What beady-eye owns

- `bdi`, the terminal view that joins a beads tracker to a herdr session.
- The reading side: which `bd`, `herdr` and `gh` command lines it spells, and
  how it parses what they print.
- The two pseudopods. One carries a person's answer to the bead that asked.
  The other settles a gh:pr gate: it re-reads the pull request from GitHub,
  closes the gate on a merge, and on a close without one tells the beads the
  gate holds back.
- The model that joins the two sources, including the anomalies where they
  disagree, and the words and colours `bdi` draws.
- The `bdi` configuration format: `[[badges]]`, `join.pane_key` and the rest.
- The `herdr-plugin.toml` that installs it as a herdr plugin, and the releases
  published to crates.io and GitHub.

## What it does not own

- **Beads.** The tracker, its schema and its commands belong to the beads
  project. `bdi` reads from it, and a change to what `bd` prints is asked of
  beads.
- **herdr.** Panes, agent detection and agent status belong to the herdr
  project. `bdi` draws what herdr reports.
- **Every other write to a tracker.** `bdi` never creates a bead, and it
  writes only through its pseudopods. `bdi bd`'s `human respond` records a
  person's answer and closes the bead that asked. Settling a gh:pr gate closes
  the gate or comments on the beads it holds back. Whoever runs `bdi` does
  everything else with `bd`, creating the gate included.
- **How agents are organised.** `bdi` knows no roles, orchestration model or
  skill names. A convention a setup keeps in bead metadata is named in the
  user's config and drawn without interpretation.
- **A user's deployment.** Their tracker, its server, their config and their
  data live with whoever runs `bdi`.

## Building and testing

`nix develop` gives you the toolchain: `cargo`, `clippy`, `rustfmt` and
`rust-analyzer`. `cargo build` and `cargo test` work as usual inside the shell.

`nix flake check` is the whole of CI, and `check-before-push` is what to run
before you push: it runs the check, and refuses a dirty tree rather than check a
source nix cannot see all of.

`plugin/` is the Claude Code plugin's TypeScript workspace, and the dev shell
gives `bun`, `biome` and `tsc` for it. `nix flake check` runs its typecheck,
lint and tests. After changing `plugin/bun.lock`, the fixed-output hash of
`pluginModules` in `flake.nix` changes too: set it to `pkgs.lib.fakeHash` and
copy the hash the failed build reports.

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
cannot observe it, and passes. What says a window is up is `window_over`
answering at all, and what says *which* bead it is over is the id it hands
back.

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

**Degrade, never disappear.** An unreachable tracker, a filtered tree, an
orphaned dependency — each is reported, never silently dropped.
