# beady-eye — working notes

`bdi` joins a beads tracker to a herdr session and draws one tree of work per
root, annotated with the live agent on each node. Read-only.

## Where things are

- `docs/design.md` is the spec. It is accepted; changes to it are design
  decisions, not edits.
- `docs/plans/2026-08-30-core-and-json.md` is the implementation plan for the
  core and `--json`. Its **Known defects in this plan** section at the end is a
  fix-as-you-reach-it list, not a backlog — each entry names the task it lands
  in.
- The TUI gets its own plan, written against the real types once the core
  compiles. The Noctalia widget waits behind the JSON contract.

No code exists yet. The crate, the flake and `cargo` arrive with Task 1 of the
plan.

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

The tracker is live: prefix `bdi`, database `beady-eye` on
`tracker.example.invalid:3306`, a tenant on a shared Dolt
server. Schema v53.

Use the dev shell — `nix develop`, or direnv. It pins bd 1.2.2 and scopes
`BEADS_DIR` to this repo, so a bare `bd` from an ambient shell may resolve a
different binary or another project's tracker. `.envrc` loads `.env.local` for
`BEADS_DOLT_PASSWORD` (0600, gitignored); recover it from the cluster with
`kubectl --context admin@cluster -n dolt get secret tracker-sql-users -o jsonpath='{.data.beady-eye}' | base64 -d`.

bd's auto-backup is off: the tenant SQL user is DB-scoped and cannot register a
server-side backup remote. Recovery is the cluster DB's own nightly backup.

Packaging `bdi` into the NixOS config is tracked separately, in that project, as
`nix-b8et4`.

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
