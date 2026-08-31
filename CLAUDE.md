# beady-eye — working notes

`bdi` joins a beads tracker to a herdr session and draws one tree of work per
root, annotated with the live agent on each node. Read-only.

## Where things are

- `docs/design.md` is the spec, and a starting point rather than gospel — we
  deviate from it as we learn. Where it is ambiguous, the seat that hits the
  ambiguity decides it against the code in front of it and records what it
  chose; `bdi-2bb.1` folds those decisions back into the document at the end.
- `docs/plans/2026-08-30-core-and-json.md` is the implementation plan for the
  core and `--json`. Its **Known defects in this plan** section at the end is a
  fix-as-you-reach-it list, not a backlog — each entry names the task it lands
  in.
- The TUI gets its own plan, written against the real types once the core
  compiles. The Noctalia widget waits behind the JSON contract.

## CI

`nix flake check` is the whole of CI, and it runs on homelab's own ARC runners.
Everything it runs comes from the flake's `checks` output, so a check added
there is a check CI runs.

Four of its seven minutes compile `bd`, because `tests/no_config.rs` runs the
binary as a fresh machine would and has to tell "bd present, no tracker here"
apart from "bd not installed". The workflow seeds that build into the shared
attic cache so the next run substitutes it instead of compiling it again.
Retention there is 30 days, so a month of quiet drops the seed and the next run
pays the four minutes and re-seeds. That is the first thing to check if CI is
suddenly slow again.

## Rules this project was designed under

**Terminology comes from beads or herdr.** Never invent a word where either
project has the concept. Where both are silent, coin one and add it to the
terminology table in `docs/design.md` marked *coined*. Today that table holds
exactly two: `badge` and `unattributed`.

**No coupling to any agent workflow.** `bdi` knows nothing about how agents are
organised — no roles, no orchestration model, no skill names. A convention a
setup encodes in bead metadata is named in config (`[roots]`, `[[badges]]`,
`join.pane_key`) and drawn without interpretation. If a feature needs to know
what a metadata key *means*, it belongs in config, not in the model.

**The key is `(project, id)`.** Bead prefixes are per-tracker and uncoordinated.

**Degrade, never disappear.** An unreachable tracker, a filtered tree, a dangling
parent, a truncated subtree — each is reported, never silently dropped.

## Working in this repo

Fixtures under `tests/fixtures/` are the shapes of `bd dep tree … --json` and
`herdr agent list`, captured from a live session on 2026-08-30. To add a fixture,
capture it from **this machine's own** session or trackers this project's seat can
reach — do not read another project's `.beads` directory or source tree. Ask
through that project's commy channel instead.

## Tracker and packaging

The maintainers track work in a [bd (beads)](https://github.com/gastownhall/beads)
tracker that is not part of this repository — external contributors don't need
it and should use GitHub issues instead. `.beads/` is gitignored, and nothing
tracked here names the tracker, its server or its credentials.

A maintainer opts in with an untracked `.envrc.local` holding
`use flake .#maintainer`. That shell adds the pinned `bd` and scopes
`BEADS_DIR` to this repo, so a bare `bd` from an ambient shell cannot resolve a
different binary or another project's tracker; the password comes from an
untracked `.env.local`, 0600. Server coordinates, how to recover that password,
and the tracker's operational notes live in `CLAUDE.local.md`, untracked
alongside them.

`bd` is in the contributor path too, but only as a build input: `nix flake
check` needs the binary because `tests/no_config.rs` runs `bdi` as a fresh
machine would. That is the tool under test, not a tracker.

Packaging `bdi` into the NixOS config is tracked separately, in that project.

## Names, checked 2026-08-30

`beady` is taken on crates.io. `beady-eye`, `beadyeye` and `bdi` were free.
Nothing is published, so re-check before publishing.

## Open, for Graeme

- **Prefix collisions across trackers.** `bdi` reads several trackers at once, so
  a duplicate prefix is its problem in a way it is not for a single-tracker tool.
  The design already keys on `(project, id)`, so nothing breaks, but the display
  gets confusing. Homelab confirmed `bdi` is clear of `hl`; the other projects
  have not been asked.
- **A shared read-only reader.** Homelab measured the shape: a SELECT-only MySQL
  user reaches the base tables *and* the `ready_issues` view (no `DEFINER`, so it
  resolves with invoker privileges), and is refused create / insert / update /
  delete. So a shared reader is `GRANT SELECT ON <db>.*` per tracker — a proven
  shape, not new design. It needs each project's consent before it is created,
  which is Graeme's call. Until then `credential_command` per project is the
  design.
