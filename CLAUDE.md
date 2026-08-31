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

`rerun-bdi-on-change` starts `bdi` and puts it back whenever the source
changes, so a copy left running in a terminal stays current without being
quit and started again by hand.

### Staying in your own tree

Take a worktree per seat, off `origin/main`, and run `bdi` from it too: a
project's territory is all of its working trees, so a `bdi` started in a
worktree sees the panes sitting in the shared checkout. Graeme keeps his own
session running there, so a seat that works in the shared checkout contests his
terminal for nothing.

No crate-wide `cargo fmt` while other seats are live.

Fixtures under `tests/fixtures/` are faithful captures of what `bd list`, `bd
dep tree`, `bd query` and `herdr agent list` put on the wire. To add one,
capture it from **this machine's own** session, or a tracker this project's seat
can reach — do not read another project's `.beads` directory or source tree. Ask
through that project's commy channel instead.

### Getting a change landed

Commit before you run `nix flake check`. Untracked files are invisible to it, so
a green check on a dirty tree has not compiled your new files, and `Git tree is
dirty` is the only warning you get.

Keep up to date with `origin/main` as you go, and squash onto it when you are
done.

Mutation-test before you trust a green — every seat that has done so found a
real hole. `--in-diff` scopes it to your own change, which is the difference
between two minutes and unrunnable, and it wants a cap: `systemd-run --user
--scope -p MemoryMax=4G -p MemorySwapMax=0`. The swap cap is the load-bearing
half — a non-terminating mutant reached 15.7 GiB, and with swap left available
`MemoryMax` alone pushes it there instead of killing it. `ulimit -v` is the
wrong tool: it caps address space, and the false kills it produces read exactly
like killed mutants. cargo-mutants is not in the flake — `nix run
nixpkgs#cargo-mutants -- mutants --in-diff <diff>` (`bdi-7ao.11`).

`cancelled` is the third CI answer. A superseded push leaves a run `completed /
cancelled`, which is neither green nor red, so assert `conclusion == "success"`
against the run's own `headSha` — never the absence of a failure, and never
`--limit 1`.

### Where a tool answers confidently and wrongly

`bd close --reason` is write-once. On a closed bead it echoes your text with a ✓
and stores nothing — reopen, close, and read the field back.

A test about colour asks `painted()` in `view/draw.rs`. Its neighbour `drawn()`
reads `symbol()` only and is blind to styling, which is how a colour bug
shipped. `tui.rs` has a `painted()` of its own that is no better: it is named
for the `paint()` it calls, returns symbols, and sees no colour either.

`assert!(drawn[0].contains(phrase::truncated()))` asks the function that drew
the row what the row should say, so it holds for whatever words `phrase::`
returns, and `contains("")` is true of every string, so an emptied phrase leaves
it unable to fail at all. The phrases asserted that way are the ones carrying
*degrade, never disappear* — the unreachable tracker, the root with no rows, the
truncated subtree — so the rule ends up resting on the assertions least able to
check it. Write the words the reader has to see.

Ask the program, not the library under it. `fc-match` says `\e[1m` gets Bold;
kitty resolves it to SemiBold, and `kitty +runpy` is what will tell you so. If
you supplied part of the query, you specified the answer.

A path with no non-test caller is described by its tests, not covered by them.
`Forest::refresh` was correct, tested twice, and called by nothing for days.

## Tracker and packaging

The maintainers track work in a [bd (beads)](https://github.com/gastownhall/beads)
tracker that is not part of this repository — external contributors don't need
it and should use GitHub issues instead. `.beads/` is gitignored, and nothing
tracked here names the tracker, its server or its credentials.

A maintainer opts in with an untracked `.envrc.local` holding
`devshell=maintainer` — a shell name, not a `use flake` call. `.envrc` runs
`use flake` exactly once, on that name, because nix-direnv deletes every
profile in `.direnv` before writing its own: a second call anywhere in the
chain evicts the first and both rebuild on every load. The maintainer shell
adds the pinned `bd` and scopes `BEADS_DIR` to this repo, so a bare `bd` from
an ambient shell cannot resolve a different binary or another project's
tracker; the password comes from an untracked `.env.local`, 0600. Server
coordinates, how to recover that password, and the tracker's operational notes
live in `CLAUDE.local.md`, untracked alongside them.

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
