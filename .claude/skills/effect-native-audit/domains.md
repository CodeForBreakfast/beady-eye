# Effect domain & smell checklist

Human-readable mirror of the sweep's **four axes**. **The authoritative copies are
the `DOMAINS`, `STRUCTURAL_SMELLS`, `MODELLING_SMELLS`, and `BEHAVIOUR_SMELLS`
arrays in `effect-native-audit.workflow.js`.** The workflow script cannot read
files, so it drives the finders from those arrays directly. This page exists so a
human can review and refresh the sets without reading JS. Keep them in sync.

The first three axes are **static**: they catch non-native code that is *present*.
The fourth is **dynamic**: it catches native-but-misused or policy code.

- **Substitution axis** (`DOMAINS`) — one finder per Effect module. Each holds
  that module's *entire* export inventory, because you cannot spot that our
  helper duplicates `Array.partition` unless the whole `Array` surface is in
  front of you. *Where did we hand-roll a helper?* (verbs)
- **Structural axis** (`STRUCTURAL_SMELLS`) — one finder per shape anti-pattern.
  Each reads the Effect *design* docs (not a module inventory) and greps for code
  whose construction / DI / error / resource spine is imperative OOP even though
  the leaves return Effect. *Effect used inside functions ≠ an Effect-native
  program.* (wiring)
- **Modelling axis** (`MODELLING_SMELLS`) — one finder per representation
  anti-pattern. Each reads the `code-style/*` + `data-types/*` design docs and
  greps for data or state whose *type* throws away a guarantee an Effect data type
  would give for free: null over `Option`, sentinels over `Either`, flags +
  optional fields over a tagged union, bare primitives over branded types,
  in-place mutation over immutable structures, hand-rolled equality over `Equal`,
  data-first helpers that block clean `pipe` composition. *Is the representation
  itself honest?* (nouns)
- **Behaviour axis** (`BEHAVIOUR_SMELLS`) — one finder per dynamic anti-pattern.
  Each reads the `concurrency/*` + `error-management/*` + `schema/*` design docs
  and greps for native-but-misused or policy code: how effects **run** (sequential
  when they could be concurrent, unbounded fan-out), **fail** (untyped or swallowed
  error channel), and what they **trust** (untrusted data crossing the edge with no
  `Schema` decode). Each finder anchors on *present* code (an `Effect.all`/`fork`,
  an `as`/`JSON.parse`, a typed `E`), not blanket absence. *Does it run, fail, and
  trust the way a great Effect program should?* (dynamics)

## Why two sources

- **Source** — the *what*: the complete export inventory, signatures and JSDoc
  `@example`. It is read from `plugin/node_modules`, which `bun install` fills with
  the versions `plugin/package.json` pins, so the inventory is the installed
  Effect's. Each package ships its TypeScript under `src/`. Without
  `node_modules`, a finder fetches the pinned version from unpkg. It never reads
  Effect's GitHub `main`, which is a different major version. Effect re-exports
  many names inside `export { … }` blocks, some renamed (`_void as void`), so the
  inventory reads those blocks as well as the `export const` lines.
  Non-negotiable for substitution: grep-then-guess misses the copy-of-behaviour
  cases that matter most.
- **Docs** — the *when & why*, from Effect 3's docs. `Effect-TS/website`'s main
  branch and effect.website now document Effect 4. The Effect 3 docs are kept at
  the tag `pre-website-v2-migration`, under `content/src/content/docs/docs/`. A
  slug maps to `<slug>.mdx` (a page) or `<slug>/` (a multi-page section the finder
  reads in full, not just the intro). For substitution, the docs separate a real
  swap from a false twin. For structural, modelling, and behaviour the design docs
  ARE the ground, since the smell is a shape, a representation, or a dynamic
  policy, not an export. Where the docs and the installed source disagree, the
  source wins.

## Substitution domains

`Docs` is a slug under the docs root: a page or a section directory. `—` means the
module is API-reference only, with no prose page, so the finder leans on source.
Source paths are under `plugin/node_modules/`.

| Domain | Source module(s) | Docs slug |
|---|---|---|
| `Array` | `effect/src/Array.ts` | — |
| `Record` | `Record.ts`, `Struct.ts` | — |
| `Chunk` | `Chunk.ts` | data-types/chunk |
| `Option` | `Option.ts` | data-types/option |
| `Either` | `Either.ts` | data-types/either |
| `Predicate` | `Predicate.ts`, `Function.ts` | getting-started/building-pipelines |
| `String` | `String.ts` | — |
| `Number` | `Number.ts` | — |
| `Effect` | `Effect.ts` | getting-started/control-flow |
| `Config` | `Config.ts`, `ConfigProvider.ts` | configuration |
| `Schema` | `Schema.ts` | schema/ (section) |
| `Schedule` | `Schedule.ts`, `Cron.ts` | scheduling/ (section) |
| `Stream` | `Stream.ts` | stream/ (section) |
| `Platform` | `@effect/platform/src/` `FileSystem.ts`, `Path.ts`, `Socket.ts`, `Command.ts`, `Terminal.ts`; `@effect/platform-node/src/` `NodeSocket.ts`, `NodeStream.ts`, `NodeRuntime.ts` | platform/ (section) |
| `Layer` | `Layer.ts`, `Context.ts` | requirements-management/ (section) |
| `Match` | `Match.ts` | code-style/pattern-matching |
| `Equal` | `Equal.ts`, `Order.ts`, `Hash.ts`, `Data.ts` | trait/ (section) |
| `Ref` | `Ref.ts`, `SynchronizedRef.ts`, `STM.ts` | state-management/ (section) |
| `Duration` | `Duration.ts`, `Clock.ts`, `DateTime.ts` | data-types/duration |
| `Cause` | `Cause.ts`, `Exit.ts` | error-management/two-error-types |
| `Concurrency` | `Queue.ts`, `PubSub.ts`, `Mailbox.ts`, `Deferred.ts`, `Fiber.ts` | concurrency/ (section) |
| `Scope` | `Scope.ts` | resource-management/ (section) |

Unless a row says otherwise, a module is under `effect/src/`. The `Platform` row
covers the Node surfaces the plugin touches: files, paths, a unix socket, stdio
and subprocesses.

## Structural smells

Each finder greps an anchor pattern, reads the design doc for the native shape
and its justified exceptions, then confirms each candidate by reading it.

| Smell | What it is | Native shape | Docs slug |
|---|---|---|---|
| `oop-construction` | `new`-able class deriving state, methods return Effect | `make` Effect, or `Context.Tag`+`Layer`/`Effect.Service` | requirements-management/ |
| `manual-di` | dependency as constructor arg / config field / **threaded param** (incl. a `ConfigProvider`/env/clock/client source parameterised so prod & tests share one path) | declare in `R` via `yield* Tag`; **provide a real Layer in prod and a fixture Layer at the test boundary** — `ConfigProvider.fromEnv`-in-prod / `fromMap`-in-tests, never threaded | requirements-management/ |
| `throw-in-effect` | `throw` / throwing brand / `*Sync` inside an Effect-returning fn | `Effect.fail` / `Data.TaggedError` / `Schema.decodeUnknown` | error-management/expected-errors |
| `internal-bridge` | `run*`/`tryPromise`/`decodeUnknownSync` at an internal seam | run only at an app edge: `NodeRuntime.runMain`, or a callback the MCP SDK invokes that must return a Promise | getting-started/running-effects |
| `mutable-state` | reassigned `let`/field as state across an Effect boundary | `Ref` / `SynchronizedRef` | state-management/ |
| `imperative-lifecycle` | `try/finally` or explicit `.close()` for cleanup | `acquireRelease` / `Scope` / Layer finalizer | resource-management/ |

The framing doc for the whole axis is `getting-started/the-effect-type`: effects
are lazy *descriptions*, not eagerly-executed state-deriving constructors.

## Modelling smells

Same shape as the structural finders: grep an anchor, read the design doc for the
honest representation and its justified exceptions, confirm by reading the site.
They hunt the *type* of data and state rather than the effect, DI or resource
spine. Verify defaults to **surfaced**, as structural does: a surfaced false
positive is cut cheaply when findings are reviewed, while a refuted true smell is
invisible. Each finding carries `blastRadius`, because a representation change
ripples to every producer and consumer of the value.

| Smell | What it is | Native representation | Docs slug |
|---|---|---|---|
| `nullable-return` | `T \| null` / `\| undefined` as "maybe absent" | `Option<T>` (`Option.fromNullable` + combinators) | data-types/option |
| `sentinel-or-throw` | sentinel (`-1`/`''`/`null`) or `throw` for expected absence in a **pure/sync** fn | `Option` / `Either` | data-types/either |
| `flag-stringly-state` | booleans + optional fields, or ad-hoc string tags, allowing illegal states | `Data.taggedEnum` + `Match.exhaustive` | data-types/data |
| `bare-primitive` | domain value as bare `string`/`number`, constraints enforced ad hoc | branded / refined type (`Schema.brand`, `Brand`, `Schema.NonEmptyString`…) | code-style/branded-types |
| `in-place-mutation` | `push`/`splice`/`sort`/property writes building **pure data** | immutable build / persistent `Chunk`/`HashMap` / `Data.struct` / `readonly` | data-types/chunk |
| `hand-equality` | field-by-field or `JSON.stringify` equality / stringified dedup | `Equal.equals` + `Data.struct`/`case` (value equality + `Hash`) | trait/equal |
| `data-first-helper` | helper wrapped in `(x) => f(cfg, x)` inside `pipe` because its signature is data-first | data-last (curried) signature / `dual` — clean `pipe` composition | code-style/dual |

Three smells deliberately border an existing axis. Each finder states its
boundary so they don't double-count, and synthesis still dedups the same
`file:line`:

- `sentinel-or-throw` vs structural `throw-in-effect` — pure/sync fn vs a `throw`
  *inside* an Effect-returning fn. Effect-returning ⇒ structural; pure ⇒ modelling.
- `in-place-mutation` vs structural `mutable-state` — mutation of pure data vs
  reassigned state *crossing* an Effect/async boundary. Crosses ⇒ structural.
- `hand-equality` / `nullable-return` vs the `Equal` / `Option` substitution
  domains — a representation that *discards a guarantee* (modelling) vs a
  *reimplemented helper* (substitution). They propose different work.

`data-first-helper` serves clean `pipe` composition, **not** currying for its own
sake. The finder and the refuter both reject a data-last rewrite that would be
gratuitous point-free: a helper never piped, always fully applied at one site, or
one that reads worse.

## Behaviour smells

The **dynamic** axis: native-but-misused or policy code — how effects **run**,
**fail**, and what they **trust**. It is smell-first like structural and
modelling, with default-**surfaced** verify and `blastRadius` required. Each
finder anchors on a *present* construct (an `Effect.all`/`fork`, an
`as`/`JSON.parse`, a typed `E`) and confirms a concrete misuse. It does **not**
scan for blanket absence ("no timeout anywhere"), which is why timeout, retry and
observability gaps are deliberately *not* here. For concurrency smells, the data
dependency between effects is the whole question and must be checked before
flagging.

| Smell | What it is | Native behaviour | Docs slug |
|---|---|---|---|
| `sequential-not-concurrent` | independent effects run one-after-another (latency = sum, not max) | `Effect.all`/`forEach` with `{ concurrency }` — *only* when genuinely independent | concurrency/basic-concurrency |
| `unbounded-fanout` | `concurrency:"unbounded"` / fork-per-item over an externally-sized collection on a shared resource | a **bounded** `{ concurrency: n }` / `Effect.makeSemaphore` | concurrency/basic-concurrency |
| `unsupervised-fork` | bare `Effect.fork` — fiber not scoped/joined, failures + interruption lost | `forkScoped` / `forkDaemon` with failures observed; often `Effect.all`/`race` | concurrency/fibers |
| `unvalidated-boundary` | untrusted external data via `as`/`JSON.parse` with **no** decode | `Schema.decodeUnknown` at the edge | schema/getting-started |
| `untyped-error-channel` | `E` is `unknown`/string, or `catchAll`/`orDie`/`ignore` swallows a recoverable typed failure | tagged `Data.TaggedError` union + `catchTag`/`Match`; handle, don't downgrade | error-management/two-error-types |

Three smells border an existing axis. Each finder states its boundary so they
don't double-count, and synthesis still dedups the same `file:line`:

- `unvalidated-boundary` vs structural `internal-bridge` vs modelling
  `bare-primitive` — *no decode at all* at the trust boundary (behaviour) vs *a
  `*Sync` decode bridge that exists* at a seam (structural) vs *a domain value
  lacking a brand* (modelling).
- `untyped-error-channel` vs structural `throw-in-effect` — the typed `E` *shape
  and handling* (behaviour) vs a raw `throw` escaping into a defect (structural).
- `unsupervised-fork` vs structural `mutable-state` — fiber *lifetime*
  (behaviour) vs shared state crossing an Effect boundary (structural).

## Out of scope

- **What the plugin's linters already report.** `nix flake check` typechecks
  `plugin/` with Effect's tsgo build of `tsc`, and the plugin's `tsconfig.json`
  makes every Effect diagnostic fail it, suggestions included. Its default-on
  rules that border this sweep are `floatingEffect`,
  `missingStarInYieldEffectGen`, `unnecessaryEffectGen`, `effectSucceedWithVoid`,
  `effectMapVoid`, `returnEffectInGen`, `schemaSyncInEffect`,
  `tryCatchInEffectGen`, `runEffectInsideEffect`, `catchAllToMapError`,
  `catchToOrElseSucceed`, `catchUnfailableEffect`, `allOfMapToForEach`,
  `globalErrorInEffectFailure`, `globalErrorInEffectCatch` and
  `unknownInEffectCatch`. The Biome GritQL plugin
  `plugin/biome-plugins/effect-native-predicates.grit` lints `x instanceof Error`
  and a switch default holding a `const _: never` guard. The workflow's
  `LINT_OWNED` carries both lists to every finder.
- **Not out of scope: the global-API rules.** `globalDate`, `globalConsole`,
  `globalFetch`, `globalTimers`, `globalRandom`, `processEnv`,
  `preferSchemaOverJson`, `newPromise`, `asyncFunction`, `nodeBuiltinImport`,
  `extendsNativeError` and `anyUnknownInErrorContext` are off by default, so the
  sweep reports what they would.
- **Pure absence.** "This I/O has no `timeout`/`retry`" or "no
  `withSpan`/structured logging anywhere" gives a swap or smell detector no
  present construct to anchor on, and blanket-absence scanning is too noisy. The
  behaviour axis flags absence only when it sits on a present anchor, such as an
  untrusted value being cast or a typed `E` being swallowed. A resilience or
  observability audit would be a sibling skill.
- **Tests.** The sweep reads the non-test TypeScript under `plugin/src/` and
  `plugin/hooks/`. It skips `*.test.ts`, modules only tests import, and
  `node_modules`. Test-idiom quality, such as `TestClock` and layer-swapped
  doubles, is a real blind spot this audit does not cover.

## Refresh when bumping Effect or effect-tsgo

When `effect` or an `@effect/*` package in `plugin/package.json` changes:

1. Re-check each source path exists under `plugin/node_modules` after
   `bun install`. Modules get split or renamed across majors.
2. Re-check each docs slug resolves for that version's docs. A new major needs
   its own docs root, and the tag above holds Effect 3's.
3. Add rows for newly-significant modules or smells. The lists are a seed, not a
   cage.
4. Edit the `DOMAINS` / `STRUCTURAL_SMELLS` / `MODELLING_SMELLS` /
   `BEHAVIOUR_SMELLS` arrays in `effect-native-audit.workflow.js` and these
   tables together.

When the effect-tsgo version in `flake.nix` changes, or the plugin's
`tsconfig.json` changes its Effect diagnostics, re-measure which rules are on.
Copy `plugin/` to a scratch directory, add a file that trips each rule named
above, run `tsc --noEmit -p .` there, and update `LINT_OWNED` and the list above
to match what fires. When the grit file gains or drops a rule, update both too.
