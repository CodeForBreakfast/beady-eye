# Effect domain & smell checklist

Human-readable mirror of the sweep's **four axes**. **The authoritative copies are
the `DOMAINS`, `STRUCTURAL_SMELLS`, `MODELLING_SMELLS`, and `BEHAVIOUR_SMELLS`
arrays in `effect-native-audit.workflow.js`.** The workflow script cannot read
files, so it drives the finders from those arrays directly. Keep them in sync.

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
  would give for free: null over `Option`, sentinels over `Result`, flags +
  optional fields over a tagged union, bare primitives over branded types,
  in-place mutation over immutable structures, hand-rolled equality over `Equal`,
  data-first helpers that block clean `pipe` composition. *Is the representation
  itself honest?* (nouns)
- **Behaviour axis** (`BEHAVIOUR_SMELLS`) — one finder per dynamic anti-pattern.
  Each reads the `concurrency/*` + `error-management/*` + `schema/*` design docs
  and greps for native-but-misused or policy code: how effects **run** (sequential
  when they could be concurrent, unbounded fan-out), **fail** (untyped or swallowed
  error channel), and what they **trust** (untrusted data crossing the edge with no
  `Schema` decode). Each finder anchors on *present* code (an `Effect.all`/fork,
  an `as`/`JSON.parse`, a typed `E`), not blanket absence. *Does it run, fail, and
  trust the way a great Effect program should?* (dynamics)

## Why three sources

- **Source** — the *what*: the complete export inventory, signatures and JSDoc
  `@example`. It is read from `plugin/node_modules`, which `bun install` fills with
  the versions `plugin/package.json` pins, so the inventory is the installed
  Effect's. Each package ships its TypeScript under `src/`. Without
  `node_modules`, a finder fetches the version `plugin/bun.lock` resolves from
  unpkg. It never reads
  Effect's GitHub `main`, which moves on past the pinned version. Effect
  re-exports many names inside `export { … }` blocks, some renamed
  (`catch_ as catch`), and `@effect/platform-node` mostly re-exports
  `@effect/platform-node-shared` with `export *`. The inventory reads all three
  forms as well as the `export const` lines.
- **Idiom guide** — how Effect 4 code is written. The installed `effect` package
  ships `AGENTS.md`, which is Effect-TS/effect's `LLMS.md`, and the worked
  examples under `ai-docs/src/`, so the guide is the pinned version's. A refuter
  checking how Effect 4 changed an idiom reads Effect-TS/effect's `migration/`
  notes at the tag `effect@<version>`.
- **Docs** — the *when & why*, from Effect 4's docs. effect.website serves them
  under `/docs/v4/`, and its unprefixed pages are still Effect 3's.
  `Effect-TS/website` keeps the Effect 4 pages on `main` under
  `apps/web/src/content/docs/v4/`. A slug maps to `<slug>.mdx` (a page) or
  `<slug>/` (a multi-page section the finder reads in full, not just the intro).
  For substitution, the docs separate a real swap from a false twin. For
  structural, modelling, and behaviour the design docs ARE the ground, since the
  smell is a shape, a representation, or a dynamic policy, not an export. Where
  the docs and the installed source disagree, the source wins.

## Substitution domains

`Docs` is a slug under the docs root: a page or a section directory. `—` means the
module is API-reference only, with no prose page, so the finder leans on source.
Source paths are under `plugin/node_modules/`.

| Domain | Source module(s) | Docs slug |
|---|---|---|
| `Array` | `effect/src/Array.ts` | — |
| `Record` | `Record.ts`, `Struct.ts` | — |
| `Chunk` | `Chunk.ts` | data-types/chunk |
| `Option` | `Option.ts`, `UndefinedOr.ts` | data-types/option |
| `Result` | `Result.ts` | data-types/result |
| `Predicate` | `Predicate.ts`, `Function.ts`, `Filter.ts` | getting-started/building-pipelines |
| `String` | `String.ts` | — |
| `Number` | `Number.ts` | — |
| `Effect` | `Effect.ts` | code-style/control-flow |
| `Config` | `Config.ts`, `ConfigProvider.ts` | configuration |
| `Schema` | `Schema.ts`, `SchemaTransformation.ts`, `SchemaGetter.ts` | schema/ (section) |
| `Schedule` | `Schedule.ts`, `Cron.ts` | scheduling/ (section) |
| `Stream` | `Stream.ts` | stream/ (section) |
| `Platform` | `FileSystem.ts`, `Path.ts`, `socket/Socket.ts`, `process/ChildProcess.ts`, `Stdio.ts`, `Terminal.ts`; `@effect/platform-node-shared/src/` `NodeSocket.ts`, `NodeStream.ts`, `NodeRuntime.ts`; `@effect/platform-node/src/NodeServices.ts` | platform/ (section) |
| `Layer` | `Layer.ts`, `Context.ts` | requirements-management/ (section) |
| `Match` | `Match.ts` | code-style/pattern-matching |
| `Equal` | `Equal.ts`, `Order.ts`, `Hash.ts`, `Data.ts` | trait/ (section) |
| `Ref` | `Ref.ts`, `SynchronizedRef.ts`, `SubscriptionRef.ts` | state-management/ (section) |
| `Duration` | `Duration.ts`, `Clock.ts`, `DateTime.ts` | data-types/duration |
| `Cause` | `Cause.ts`, `Exit.ts` | error-management/two-error-types |
| `Concurrency` | `Queue.ts`, `PubSub.ts`, `Deferred.ts`, `Latch.ts`, `Semaphore.ts` | concurrency/ (section) |
| `Fiber` | `Fiber.ts`, `FiberSet.ts`, `FiberMap.ts`, `FiberHandle.ts` | concurrency/fibers |
| `Scope` | `Scope.ts` | resource-management/ (section) |

Unless a row says otherwise, a module is under `effect/src/`. Effect 4 folded
`@effect/platform` into `effect`, so the `Platform` row's services come from
`effect` and only their Node layers from `@effect/platform-node`. The row
covers the Node surfaces the plugin touches: files, paths, a unix socket, stdio
and subprocesses.

## Structural smells

Each finder greps an anchor pattern, reads the design doc for the native shape
and its justified exceptions, then confirms each candidate by reading it.

| Smell | What it is | Native shape | Docs slug |
|---|---|---|---|
| `oop-construction` | `new`-able class deriving state, methods return Effect | `make` Effect (`Effect.fn` when it takes arguments), or a `Context.Service` class with `make` and a `Layer.effect` layer | requirements-management/ |
| `manual-di` | dependency as constructor arg / config field / **threaded param** (incl. a `ConfigProvider`/env/clock/client source parameterised so prod & tests share one path) | declare in `R` via `yield* Service`; **provide a real Layer in prod and a fixture Layer at the test boundary** — the default `fromEnv` in prod, `ConfigProvider.layer(ConfigProvider.fromUnknown(…))` in tests, never threaded | requirements-management/ |
| `throw-in-effect` | `throw` / throwing brand / `*Sync` inside an Effect-returning fn | `Effect.fail` or `yield*` of a `Schema.TaggedError` / `Data.TaggedError`, `Schema.decodeUnknownEffect` | error-management/expected-errors |
| `internal-bridge` | `run*`/`tryPromise`/`decodeUnknownSync` at an internal seam | run only at an app edge: `NodeRuntime.runMain`, or a callback the MCP SDK invokes that must return a Promise | getting-started/running-effects |
| `mutable-state` | reassigned `let`/field as state across an Effect boundary | `Ref` / `SynchronizedRef` | state-management/ |
| `imperative-lifecycle` | `try/finally` or explicit `.close()` for cleanup | `acquireRelease` / `Scope` / `Effect.addFinalizer` inside `Layer.effect` | resource-management/ |

The framing doc for the whole axis is `getting-started/the-effect-type`: effects
are lazy *descriptions*, not eagerly-executed state-deriving constructors.

## Modelling smells

Same shape as the structural finders: grep an anchor, read the design doc for the
honest representation and its justified exceptions, confirm by reading the site.
They hunt the *type* of data and state rather than the effect, DI or resource
spine. Verify defaults to **surfaced**, as structural does. Each finding carries
`blastRadius`, because a representation change ripples to every producer and
consumer of the value.

| Smell | What it is | Native representation | Docs slug |
|---|---|---|---|
| `nullable-return` | `T \| null` / `\| undefined` as "maybe absent", re-checked at each use | `Option<T>` (`Option.fromNullishOr` + combinators), or `UndefinedOr` where `undefined` is the only absence marker | data-types/option |
| `sentinel-or-throw` | sentinel (`-1`/`''`/`null`) or `throw` for expected absence in a **pure/sync** fn | `Option` / `Result` | data-types/result |
| `flag-stringly-state` | booleans + optional fields, or ad-hoc string tags, allowing illegal states | `Data.taggedEnum` or `Schema.TaggedUnion` + `Match.exhaustive` | data-types/data |
| `bare-primitive` | domain value as bare `string`/`number`, constraints enforced ad hoc | branded / refined type (`Schema.brand`, `Brand.nominal`/`Brand.make`, `Schema.NonEmptyString`, `Schema.check` with `Schema.isPattern`…) | code-style/branded-types |
| `in-place-mutation` | `push`/`splice`/`sort`/property writes building **pure data** | immutable build / persistent `Chunk`/`HashMap` / `readonly` | data-types/chunk |
| `hand-equality` | field-by-field or `JSON.stringify` equality / stringified dedup | `Equal.equals`, structural on plain objects and arrays in Effect 4, with `HashSet`/`HashMap` for membership | trait/equal |
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
finder anchors on a *present* construct (an `Effect.all`/fork, an
`as`/`JSON.parse`, a typed `E`) and confirms a concrete misuse. It does **not**
scan for blanket absence ("no timeout anywhere"), which is why timeout, retry and
observability gaps are deliberately *not* here. For concurrency smells, the data
dependency between effects is the whole question and must be checked before
flagging.

| Smell | What it is | Native behaviour | Docs slug |
|---|---|---|---|
| `sequential-not-concurrent` | independent effects run one-after-another (latency = sum, not max) | `Effect.all`/`forEach` with `{ concurrency }` — *only* when genuinely independent | concurrency/basic-concurrency |
| `unbounded-fanout` | `concurrency:"unbounded"` / fork-per-item over an externally-sized collection on a shared resource | a **bounded** `{ concurrency: n }` / a shared `Semaphore.make` | concurrency/basic-concurrency |
| `unsupervised-fork` | a fork whose failure nothing observes, or whose lifetime is wrong for its job (`forkChild` ends with its parent fiber, `forkDetach` never does) | `Fiber.join`/`await`, or a handler inside the forked effect; `forkChild` / `forkScoped` / `forkIn` / `FiberSet` / `forkDetach` matched to the owner; often `Effect.all`/`race` | concurrency/fibers |
| `unvalidated-boundary` | untrusted external data via `as`/`JSON.parse` with **no** decode | `Schema.decodeUnknownEffect` or `Schema.fromJsonString` at the edge | schema/getting-started |
| `untyped-error-channel` | `E` is `unknown`/string, or `Effect.catch`/`orDie`/`ignore` swallows a recoverable typed failure | tagged error union + `catchTag`/`catchReason`/`Match`; handle, don't downgrade | error-management/two-error-types |

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
  rules that border this sweep, grouped as the workflow's `LINT_OWNED` groups
  them, are:
  - generators and laziness: `floatingEffect`, `missingStarInYieldEffectGen`,
    `unnecessaryEffectGen`, `returnEffectInGen`, `missingReturnYieldStar`,
    `lazyEffect`, `lazyPromiseInEffectSync`, `promiseInEffectSuccess`,
    `effectInVoidSuccess`, `effectInFailure` and `effectFnIife`;
  - running and decoding: `runEffectInsideEffect`, `preferUnsafeConstructor`,
    `runOfExitToRunExit`, `schemaSyncInEffect`, `tryCatchInEffectGen` and
    `preferTypedSchemaDecoder`;
  - combinator swaps: `effectSucceedWithVoid`, `effectMapVoid`, `syncToSucceed`,
    `preferSucceedSomeOrNone`, `mapSomeToAsSome`, `flatMapToMap`,
    `flatMapIgnoredParamToAndThen`, `effectMapFlatten`,
    `flatMapConditionalToFilterOrFail`, `matchEffectToMapBoth`,
    `matchEffectToMatch`, `optionMatchToFromOption`, `allOfMapToForEach`,
    `raceFirstWithSleepToTimeout`, `timeoutCatchTagToTimeoutOrElse`,
    `acquireReleaseDisposable`, `abortControllerInEffect` and
    `unnecessaryFailYieldableError`;
  - error handling: `catchAllToMapError`, `catchToOrElseSucceed`,
    `catchToIgnore`, `catchDieToOrDie`, `catchRefailToTapError`,
    `catchAllTagDispatchToCatchTag`, `catchIfTagToCatchTag`, `multipleCatchTag`,
    `catchConditionalRefailToCatchIf`, `catchTagToCatchReason`,
    `catchChainToFirstSuccessOf`, `redundantMapError`, `redundantOrDie`,
    `catchUnfailableEffect`, `globalErrorInEffectFailure`,
    `globalErrorInEffectCatch` and `unknownInEffectCatch`;
  - services and layers: `leakingRequirements`, `multipleEffectProvide`,
    `provideLayerSucceedToProvideService` and `layerMergeAllWithDependencies`;
  - Schema and APIs: `schemaStructWithTag`, `schemaNumber` and `outdatedApi`.

  The Biome GritQL plugin
  `plugin/biome-plugins/effect-native-predicates.grit` lints `x instanceof Error`
  and a switch default holding a `const _: never` guard. The workflow's
  `LINT_OWNED` carries both lists to every finder.
- **Not out of scope: the global-API rules.** `globalDate`, `globalConsole`,
  `globalFetch`, `globalTimers`, `globalRandom`, `cryptoRandomUUID`,
  `processEnv`, each with its `…InEffect` twin, and `preferSchemaOverJson`,
  `newPromise`, `asyncFunction`, `nodeBuiltinImport`, `extendsNativeError`,
  `anyUnknownInErrorContext`, `schemaSync`, `unsafeEffectTypeAssertion`,
  `instanceOfSchema` and `strictEffectProvide` are off by default, so the sweep
  reports what they would.
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
   its own docs root: the one above holds Effect 4's.
3. Re-check every API the skill and the workflow name against the installed
   barrel, for example by importing `effect` under bun and testing each
   `Module.name`.
4. Add rows for newly-significant modules or smells. The lists are a seed, not a
   cage.
5. Edit the `DOMAINS` / `STRUCTURAL_SMELLS` / `MODELLING_SMELLS` /
   `BEHAVIOUR_SMELLS` arrays in `effect-native-audit.workflow.js` and these
   tables together.

When the effect-tsgo version in `flake.nix` changes, or the plugin's
`tsconfig.json` changes its Effect diagnostics, re-measure which rules are on.
Each rule's page under effect-tsgo's `docs/rules/` gives its default severity
and the Effect versions it applies to, and its Preview block is a snippet that
trips it. Copy `plugin/` to a scratch directory, add each Preview as its own
file, run `tsc --noEmit -p .` there, and update `LINT_OWNED` and the lists
above to match what fires. When the grit file gains or drops a rule, update
both too.
