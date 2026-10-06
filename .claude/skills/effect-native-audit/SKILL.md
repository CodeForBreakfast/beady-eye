---
name: effect-native-audit
description: Use when asked to audit the Claude Code plugin under plugin/ for places it hand-rolled what Effect ships natively, or to run the Effect-native sweep. Triggers on "effect-native audit", "where are we reinventing Effect", "rolled our own instead of an Effect builtin", "sweep for native Effect helpers", "audit our FP/Effect modelling", "audit our concurrency/error handling". A workflow-driven substitution, structural, modelling and behaviour audit — not a substitute for the Effect language service or the plugin's Biome rules.
---

# Effect-native capability audit

Sweep the plugin's TypeScript on **four axes** for code that isn't Effect-native.
Three axes are **static** and find non-native code that is *present*. One is
**dynamic** and finds native code that is *misused*.

- **Substitution** (verbs) — logic that re-implements what Effect already ships:
  a retry loop that is `Schedule`, manual env parsing that is `Config`, custom
  equality that is `Equal`/`Data`, a list helper already in `Array`. *Where did
  we hand-roll a helper?*
- **Structural** (wiring) — code whose construction, dependency-injection, error
  or resource **spine** is still imperative OOP even though the leaves return
  Effect: a `new`-able class doing DI, a `throw` inside an Effect-returning
  function, a dependency threaded as a config field instead of declared in `R`,
  a `try/finally` that wants `acquireRelease`. *Effect used inside functions ≠ an
  Effect-native program.*
- **Modelling** (nouns) — data and state whose **type** throws away a guarantee
  an Effect data type would give for free: `T | null` that wants `Option`, a
  sentinel or pure `throw` that wants `Either`, booleans + optional fields that
  want a `Data.taggedEnum`, a bare `string` id that wants a branded type,
  in-place array mutation that wants an immutable build, hand-rolled equality
  that wants `Equal`/`Data`, a data-first helper that blocks clean `pipe`
  composition. The dishonesty is in the *representation*, so the substitution
  and structural axes walk past it.
- **Behaviour** (dynamics) — how effects **run**, **fail**, and what they
  **trust**. Independent effects run sequentially that could run under
  `Effect.all` concurrency, an unbounded fan-out onto a shared resource, a bare
  `Effect.fork` whose fiber leaks, untrusted data crossing the edge via
  `as`/`JSON.parse` with no `Schema` decode, an `E` channel typed `unknown`, or a
  `catchAll` that swallows a recoverable failure. The construct is already
  Effect-native and the smell is the *policy*. Each finder anchors on present
  code, not blanket absence ("no timeout anywhere").

The plugin has two per-line linters, and this sweep covers what they cannot see.
`nix flake check` typechecks `plugin/` with Effect's tsgo build of `tsc`, whose
language service fails the check on floating Effects, an unnecessary
`Effect.gen`, a `*Sync` decode or a `try/catch` inside a generator, the global
`Error` in `E`, and more. The Biome GritQL plugin
`plugin/biome-plugins/effect-native-predicates.grit` lints the mechanical
substitutions precise enough to flag deterministically, and leaves the
judgment-heavy axes to this sweep. Neither sees that a 40-line block is a
combinator that exists, that a module is shaped like an OOP service, that a type
lets the caller forget the absent case, or that a fan-out is unbounded. The
finders do not re-report what either linter catches. `domains.md` lists both
sets, and the global-API rules this repo leaves off, which the sweep does report.

## How it works

A `Workflow` script (`effect-native-audit.workflow.js`) fans out four kinds of
finder (see `domains.md`), grounded in two sources pinned to the plugin's Effect:

- **Source** comes from `plugin/node_modules`, the packages `bun install` puts
  there at the versions `plugin/package.json` pins. Each ships its TypeScript
  under `src/`.
- **Docs** come from Effect 3's docs, which `Effect-TS/website` keeps at the tag
  `pre-website-v2-migration`. effect.website itself now documents Effect 4.

**Substitution finders** (~22, one per Effect domain) work inventory-first:

1. List the module's *complete* export inventory from source, including the
   names Effect re-exports inside `export { … }` blocks. You cannot recognise a
   copy of `Array.partition` unless the whole `Array` surface is in your head.
2. Read the module's docs **section** for the *when & why*: the whole section,
   not just the intro page (Stream is six pages). API-reference-only modules
   (`Array`, `Record`, …) have no prose page and lean on source.
3. Scan the plugin for logic that re-expresses a helper in that inventory.

**Structural finders** (~6, one per smell in the anti-pattern catalogue) work
smell-first. They read the Effect *design* docs (`services`, `expected-errors`,
`scope`, `running-effects`, `the-effect-type`) for why the native shape wins and
when the imperative shape is justified. Then they grep an anchor pattern and read
each candidate to confirm the shape. A grep hit is not a finding.

**Modelling finders** (~7, one per representation smell) work smell-first too,
grounded in the `code-style/*` + `data-types/*` docs (`option`, `either`,
`data`, `branded-types`, `chunk`, `trait/equal`, `dual`). They hunt the *type* of
data and state. Three of them deliberately border a structural or substitution
smell, and each finder states its boundary so the axes don't double-count.

**Behaviour finders** (~5, one per dynamic smell) work smell-first, grounded in
the `concurrency/*`, `error-management/*` and `schema/*` docs. They hunt how
effects **run** (sequential-not-concurrent, unbounded-fanout), **fail**
(untyped-error-channel), and what they **trust** (unvalidated-boundary,
unsupervised-fork). Each anchors on a *present* construct and confirms a
concrete misuse. For concurrency, the data dependency between effects is the
whole question.

Findings then flow, per finding and with no barrier, into an **adversarial
verify** pass with one refuter per finding. The axes take **opposite default
verdicts**, because their failure modes are opposite:

- **Substitution** — the refuter tries to *disprove the swap* (a false twin) and
  **defaults to refuted** when unsure. A plausible-but-wrong helper swap is the
  risk, so the bar is "positively confirm the swap preserves behaviour."
- **Structural, modelling & behaviour** — the refuter asks only *is this a false
  positive?* and **defaults to surfaced** when unsure. It refutes only a misread
  or a genuinely justified choice. Whether the rewrite is worth its blast radius
  is not a refutation ground, because a surfaced false positive is cheap to drop
  while a refuted true smell is invisible. Two refuters carry an extra guard:
  `data-first-helper` kills a data-last rewrite that would be gratuitous
  point-free, and `sequential-not-concurrent` refutes when the effects are
  genuinely data-dependent.

A synthesis agent writes the report, deduping cross-axis sightings as it writes.
The confirmed / low-confidence partition is NOT trusted to that agent. The
workflow derives it deterministically from each survivor's own confidence: dedup
findings that propose the same remedy at one `file:line`, highest confidence
wins, then split. A finding whose refuter died is returned as unverified, never as
refuted.

## Running it

1. Install the plugin's packages, so the finders read the pinned Effect source:

   ```bash
   (cd plugin && bun install --frozen-lockfile)
   ```

2. Optionally, check out the Effect 3 docs once, so ~40 finders read files
   instead of fetching pages from GitHub:

   ```bash
   git clone --depth 1 --branch pre-website-v2-migration \
     https://github.com/Effect-TS/website <dir>
   ```

3. Get today's date (`date +%F`) and launch the workflow:

   ```
   Workflow({
     scriptPath: '.claude/skills/effect-native-audit/effect-native-audit.workflow.js',
     args: { date: '<YYYY-MM-DD>', docsDir: '<dir>/content/src/content/docs/docs' },
   })
   ```

   Leave out `docsDir` to read the docs from GitHub at the tag. The workflow runs
   in the background: ~40 finders (22 substitution, 6 structural, 7 modelling,
   5 behaviour), then a refuter per finding, then synthesis. Watch it with
   `/workflows`. To run one axis, pass `axes: ['behaviour']`. `reportPath`
   overrides where the report goes.

4. The workflow writes the report to `docs/effect-native-audit-<date>.md` and
   returns `{ confirmed[], lowConfidence[], unverified[], refutedCount, coverage }`.
   `coverage` names any finder that failed, any group with unverified findings,
   and any that fell back to fetching. Re-run an incomplete group on resume.

## What comes back

The workflow returns findings and files nothing. The report is scratch for
reviewing them, not a permanent record, so it is not committed.

A finding taken into the tracker has to stand on its own once the report is
gone. Carry its `file:line`, the rewrite, the why, its `sourceRef` and its
`docsRef`. Structural, modelling and behaviour findings also carry their
`blastRadius`: the call sites a rewrite ripples to, since a representation change
moves every producer and consumer of the value. Each finding is its own change,
made separately and test-first, not as a batch.

A substitution that recurs and is mechanical enough to flag deterministically
belongs as a rule in the grit file, so the linter catches the next one.

## Refreshing it

Source paths and docs slugs drift with Effect releases, and the language-service
rules that are on drift with effect-tsgo releases. `domains.md` carries the
checklist. The authoritative sets are the `DOMAINS`, `STRUCTURAL_SMELLS`,
`MODELLING_SMELLS` and `BEHAVIOUR_SMELLS` arrays and `LINT_OWNED` in the
workflow script, and `domains.md` mirrors them for humans.
