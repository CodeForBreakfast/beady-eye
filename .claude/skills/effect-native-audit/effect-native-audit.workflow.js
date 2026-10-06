export const meta = {
  name: 'effect-native-audit',
  description:
    'Sweep the codebase on four axes. SUBSTITUTION: domain-by-domain for logic that re-implements an Effect helper a per-line linter cannot see (inventory-first finders read Effect source + docs). STRUCTURAL: for code whose construction / DI / error / resource SPINE is still imperative OOP even though the leaves return Effect. MODELLING: for data/state REPRESENTATIONS that discard a type-level guarantee Effect data types provide — null over Option, sentinels over Either, flags over tagged unions, bare primitives over branded types, in-place mutation over immutable structures, hand-rolled equality over Equal, data-first helpers that block clean pipe composition. BEHAVIOUR: for native-but-misused / policy code — how effects RUN, FAIL, and what they TRUST — sequential when independent effects could be concurrent, unbounded fan-out at a shared service, unsupervised forks, untrusted data crossing the edge with no Schema decode, an untyped/swallowed error channel. The latter three axes read the Effect design docs and grep an anti-pattern catalogue. Adversarial verify kills false twins and unjustified rewrites; synthesis writes a report and returns the confirmed findings.',
  phases: [
    { title: 'Sweep', detail: 'inventory-first finder per Effect domain (~22) + shape finder per structural smell (~6) + representation finder per modelling smell (~7) + behaviour finder per dynamic smell (~5)' },
    { title: 'Verify', detail: 'one adversarial refuter per finding — false-twin for swaps, justified-shape for structural + modelling + behaviour' },
    { title: 'Synthesise', detail: 'dedup, split confirmed vs low-confidence, write report' },
  ],
}

// ===========================================================================
// args may arrive as a parsed object or, depending on how it was passed, as a JSON string —
// normalise both so date/axes/reportPath/docsDir aren't silently dropped (which falls back to a
// full four-axis sweep named "undated").
const A = (() => {
  try {
    return typeof args === 'string' ? JSON.parse(args) : args || {}
  } catch {
    return {}
  }
})()

// ---------------------------------------------------------------------------
// Audit target: the Claude Code plugin's TypeScript, scanned once.
// Tests (*.test.ts), the helpers only tests import, and node_modules are out of scope for this pass.
// ---------------------------------------------------------------------------
const AUDIT_ROOTS = ['plugin/src', 'plugin/hooks']
const AUDIT_SCOPE = `Scan only these roots, excluding *.test.ts, any module that only tests import (such as
plugin/src/test-watcher.ts), and any */node_modules/*:
  ${AUDIT_ROOTS.join(' ')}
Report every hit by its repo-relative path (e.g. plugin/src/watcher.ts, plugin/hooks/hand-over-session.ts).`

// Effect source is read from the packages the plugin actually installed, so every inventory is the
// pinned version's and not whatever a branch happens to hold. `bun install` in plugin/ puts each
// package's TypeScript source under <package>/src/. A domain's `src` is a path under this root.
const SOURCE = 'plugin/node_modules'
const SOURCE_FALLBACK = `If ${SOURCE} is absent, read each package's pinned version from plugin/package.json and
   fetch https://unpkg.com/<package>@<version>/<path under the package> instead (e.g.
   https://unpkg.com/effect@<version>/src/Array.ts), and set fellBackToFetch=true. Never read Effect
   from GitHub's main branch or any unpinned copy: main is a different major version.`

// Effect 3's docs prose (Astro/Starlight). Effect-TS/website's main branch and effect.website now
// document Effect 4; the Effect 3 docs are kept at the DOCS_TAG tag, under DOCS_PATH. A slug maps to
// <docs>/<slug>.mdx (a page) or <docs>/<slug>/ (a multi-page section — read EVERY .mdx inside).
// A domain's `docs` is that slug, or null when the module is API-reference only (no prose page —
// e.g. Array/Record/String) and source is the ground. args.docsDir names a local checkout of
// DOCS_PATH at DOCS_TAG; without one, finders read the tag from GitHub.
const DOCS_TAG = 'pre-website-v2-migration'
const DOCS_PATH = 'content/src/content/docs/docs'
const DOCS_DIR = A.docsDir || null

// ---------------------------------------------------------------------------
// Domain checklist — AUTHORITATIVE copy. domains.md mirrors this for humans.
// Each finder holds ONE module's full export inventory in context, which is why
// helper-dense modules (Array, Effect, Option...) each get their own row.
// Refresh when bumping Effect: add/rename rows, re-check source paths + docs URLs.
// ---------------------------------------------------------------------------
const DOMAINS = [
  // helper-dense data modules — where copy-of-behaviour hides most. Array/Record/String/Number
  // are API-reference only (docs:null) — their ground is the source inventory + JSDoc.
  { key: 'Array', src: ['effect/src/Array.ts'], docs: null },
  { key: 'Record', src: ['effect/src/Record.ts', 'effect/src/Struct.ts'], docs: null },
  { key: 'Chunk', src: ['effect/src/Chunk.ts'], docs: 'data-types/chunk' },
  { key: 'Option', src: ['effect/src/Option.ts'], docs: 'data-types/option' },
  { key: 'Either', src: ['effect/src/Either.ts'], docs: 'data-types/either' },
  { key: 'Predicate', src: ['effect/src/Predicate.ts', 'effect/src/Function.ts'], docs: 'getting-started/building-pipelines' },
  { key: 'String', src: ['effect/src/String.ts'], docs: null },
  { key: 'Number', src: ['effect/src/Number.ts'], docs: null },
  // core combinators
  { key: 'Effect', src: ['effect/src/Effect.ts'], docs: 'getting-started/control-flow' },
  // structural / domain modules
  { key: 'Config', src: ['effect/src/Config.ts', 'effect/src/ConfigProvider.ts'], docs: 'configuration' },
  { key: 'Schema', src: ['effect/src/Schema.ts'], docs: 'schema' },
  { key: 'Schedule', src: ['effect/src/Schedule.ts', 'effect/src/Cron.ts'], docs: 'scheduling' },
  { key: 'Stream', src: ['effect/src/Stream.ts'], docs: 'stream' },
  // the Node surfaces the plugin touches: files, paths, a unix socket, stdio, subprocesses
  { key: 'Platform', src: ['@effect/platform/src/FileSystem.ts', '@effect/platform/src/Path.ts', '@effect/platform/src/Socket.ts', '@effect/platform/src/Command.ts', '@effect/platform/src/Terminal.ts', '@effect/platform-node/src/NodeSocket.ts', '@effect/platform-node/src/NodeStream.ts', '@effect/platform-node/src/NodeRuntime.ts'], docs: 'platform' },
  { key: 'Layer', src: ['effect/src/Layer.ts', 'effect/src/Context.ts'], docs: 'requirements-management' },
  { key: 'Match', src: ['effect/src/Match.ts'], docs: 'code-style/pattern-matching' },
  { key: 'Equal', src: ['effect/src/Equal.ts', 'effect/src/Order.ts', 'effect/src/Hash.ts', 'effect/src/Data.ts'], docs: 'trait' },
  { key: 'Ref', src: ['effect/src/Ref.ts', 'effect/src/SynchronizedRef.ts', 'effect/src/STM.ts'], docs: 'state-management' },
  { key: 'Duration', src: ['effect/src/Duration.ts', 'effect/src/Clock.ts', 'effect/src/DateTime.ts'], docs: 'data-types/duration' },
  { key: 'Cause', src: ['effect/src/Cause.ts', 'effect/src/Exit.ts'], docs: 'error-management/two-error-types' },
  { key: 'Concurrency', src: ['effect/src/Queue.ts', 'effect/src/PubSub.ts', 'effect/src/Mailbox.ts', 'effect/src/Deferred.ts', 'effect/src/Fiber.ts'], docs: 'concurrency' },
  { key: 'Scope', src: ['effect/src/Scope.ts'], docs: 'resource-management' },
]

// ---------------------------------------------------------------------------
// Structural smell catalogue — the SECOND axis. AUTHORITATIVE copy; domains.md mirrors it.
// These finders are NOT inventory-first. They hunt program SHAPE: code where the leaves return
// Effect but the construction / dependency-injection / error / resource SPINE is still imperative
// OOP. No module export is named "do not write a class that does DI", so the substitution finders
// walk straight past these. Each smell is grounded in an Effect *design* doc (the "think Effect"
// pages), not a module inventory. `docs` is a docs slug (page or section dir), same as above.
// ---------------------------------------------------------------------------
const STRUCTURAL_SMELLS = [
  {
    key: 'oop-construction',
    grep: "class \\w+ with #private fields and a new-able constructor; methods returning Effect.Effect<...>",
    smell:
      'A `new`-able class whose constructor synchronously derives state into private fields, with methods that return Effect. The leaves are Effect; the construction spine is OOP.',
    nativeShape:
      'A `make` Effect — makeX(args): Effect<X, E, R> — that does `const dep = yield* Dep` and returns a closure-record of the operations; OR Context.Tag + Layer / Effect.Service for a genuine singleton. Pick by cardinality: N instances keyed by params -> make Effect; one shared instance -> Tag+Layer. (A class is NOT wrong merely for existing — Schema classes, tagged errors, Context.Tag subclasses are idiomatic. The smell is a class used as a SERVICE/CONSTRUCTION mechanism.)',
    docs: 'requirements-management',
    nativeRef: 'Context.Tag, Effect.Service, Layer (Effect.ts / Context.ts / Layer.ts)',
  },
  {
    key: 'manual-di',
    grep: "a dependency (httpClient, clock, logger, db, client, ConfigProvider/config source, env object) carried as a constructor arg or as a field/parameter on a config/options object, resolved at one layer and threaded down — INCLUDING a provider parameterised so the SAME code path takes the real value in prod and a fixture in tests (e.g. a layer/function taking an `env`/`provider` arg fed the real source in prod and a fixture in tests)",
    smell:
      'A dependency carried as a constructor argument, config-object field, or threaded parameter instead of provided through the Effect DI mechanism (declared in the requirements R channel, provided by a Layer). Composition is manual DI threading, not `yield* Tag` / Layer provision, so the requirement is not type-enforced — the thing can be built without its dependency. This is GENERAL, not config-specific: the idiomatic way to provide ANY dependency is a real Layer in prod and a fixture Layer at the TEST boundary — never a parameter threaded through the construction path to unify the two. A telltale instance is a `ConfigProvider` (or any env / clock / client / logger source) parameterised so prod passes the real source and tests pass a fixture through the SAME function: prod should provide the real source directly (e.g. `ConfigProvider.fromEnv`, the live service Layer) and tests should override at their OWN boundary (e.g. `ConfigProvider.fromMap` via `setConfigProvider`/`withConfigProvider`, a fixture service Layer). The threaded parameter, an `as`-cast that reads the raw source in prod (`process.env as EnvLike`), and any converter that exists only to feed the unified path are all artifacts of the smell.',
    nativeShape:
      'Provide the dependency via the Effect DI mechanism, splitting prod and test at the EDGE rather than threading a parameter: prod composes the real Layer (`Layer.setConfigProvider(ConfigProvider.fromEnv())`, the live `Context.Tag` service Layer); tests override at the test boundary (`Layer.setConfigProvider(ConfigProvider.fromMap(fixture))` / `Effect.withConfigProvider`, a fixture Layer). The construction Effect.gen does `const dep = yield* DepTag`; the dependency leaves the config/constructor/parameter list; composition cannot type-check without a Layer providing it. (Canonical: a ConfigProvider threaded as an `env` parameter instead of `fromEnv`-in-prod / `fromMap`-in-tests.)',
    docs: 'requirements-management',
    nativeRef: 'Effect requirements channel (R), Context.Tag, Layer, Layer.setConfigProvider / Effect.withConfigProvider, ConfigProvider.fromEnv / fromMap',
  },
  {
    key: 'throw-in-effect',
    grep: "throw new ... | a brand/smart constructor that throws | Schema.decodeUnknownSync / *Sync — INSIDE a function whose return type is Effect.Effect<...> but OUTSIDE an Effect.gen body (a *Sync decode inside a generator already fails the typecheck as schemaSyncInEffect)",
    smell:
      'A synchronous `throw` (or a throwing brand constructor, or a *Sync schema decode) inside a function that returns an Effect. The failure escapes the typed E channel and becomes a defect instead of a tracked, recoverable error.',
    nativeShape:
      'Move the failure into E: Effect.fail with a Data.TaggedError, a branded validator returning Effect<_, ParseError>, or Schema.decodeUnknown (Effect-returning). Reserve `throw`/Effect.die for genuine defects — an invariant that indicates a programmer bug, not a runtime input problem.',
    docs: 'error-management/expected-errors',
    nativeRef: 'Effect.fail, Data.TaggedError, Schema.decodeUnknown',
  },
  {
    key: 'internal-bridge',
    grep: "Effect.runPromise | Effect.runSync | Effect.runFork | Effect.tryPromise | Schema.decodeUnknownSync — anywhere that is NOT an app edge. The plugin has two: NodeRuntime.runMain, and a callback the MCP SDK invokes that must hand back a Promise (its request handlers and lifecycle hooks). A run* directly inside an Effect.gen body already fails the typecheck as runEffectInsideEffect.",
    smell:
      'An Effect<->Promise (or sync) bridge at an INTERNAL seam: run*/tryPromise/decodeUnknownSync used mid-program to keep a method Promise- or value-returning, instead of returning Effect and letting it flow through to one run* call at the app edge. Each bridge fragments the typed E channel.',
    nativeShape:
      'Return Effect from the seam and compose upward; run* lives only at the host boundary. Where a Node API is wrapped in tryPromise, an @effect/platform service (FileSystem, Socket, Command) may already be the Effect-native edge. NOTE: tryPromise around a third-party Promise API that has no Effect counterpart (the MCP SDK) is the correct bridge, not this smell.',
    docs: 'getting-started/running-effects',
    nativeRef: 'run* only at the edge (running-effects); @effect/platform services for Node I/O',
  },
  {
    key: 'mutable-state',
    grep: "let x = ...; later x = ... reassignment, or a mutable (non-readonly) class field, used as state that crosses an Effect / async boundary",
    smell:
      'Shared or threaded state held in a reassigned `let` or a mutable class field where the value crosses an Effect/async boundary. Not referentially transparent; races under concurrency.',
    nativeShape:
      'Ref / SynchronizedRef for mutable state inside Effect, or thread the value functionally. NOTE: a pure local accumulator inside a single synchronous function that never crosses an Effect boundary is fine — do not flag those.',
    docs: 'state-management',
    nativeRef: 'Ref, SynchronizedRef',
  },
  {
    key: 'imperative-lifecycle',
    grep: "try { ... } finally { cleanup } for resource release, or an acquire paired with an explicit .close()/.dispose()/.end()/.destroy() call",
    smell:
      'Hand-rolled resource lifecycle: a try/finally or an explicit acquire-then-.close()/.dispose() pair managing a resource, instead of binding release to a scope so it is guaranteed and composes.',
    nativeShape:
      'Effect.acquireRelease + Scope, or a Layer finalizer (releases run in reverse order). Effect.scoped bounds the lifetime to the effect. Release then survives interruption and failure, which a bare finally may not.',
    docs: 'resource-management',
    nativeRef: 'Effect.acquireRelease, Scope, Layer finalizer, Effect.scoped',
  },
]

// ---------------------------------------------------------------------------
// Modelling smell catalogue — the THIRD axis. AUTHORITATIVE copy; domains.md mirrors it.
// Where substitution hunts VERBS (a hand-rolled helper) and structural hunts WIRING (an imperative
// effect/DI/resource spine), modelling hunts NOUNS: data and state REPRESENTATIONS that throw away a
// type-level guarantee Effect's data types would give for free. The smell is in the TYPE/SHAPE of the
// data, not in a missing function call and not in the effect spine — so both other axes walk past it.
// Grounded in the `code-style/*` and `data-types/*` design docs (the "model it honestly" pages).
// Three smells deliberately border an existing axis; each states its boundary so they don't double-count
// (synthesis still dedups same file:line): sentinel-or-throw vs structural throw-in-effect (pure/sync vs
// inside-Effect), in-place-mutation vs structural mutable-state (pure data vs state crossing an Effect
// boundary), hand-equality/nullable-return vs the Equal/Option substitution domains (representation that
// discards a guarantee vs a reimplemented helper). `docs` is a docs slug, same as the other axes.
// ---------------------------------------------------------------------------
const MODELLING_SMELLS = [
  {
    key: 'nullable-return',
    grep: "a function returning `T | null` / `T | undefined`, or an optional field standing for 'maybe absent', whose absence the callers branch on with == null / ?. / ?? checks",
    smell:
      'Optional presence modelled as `T | null` / `T | undefined` (return type or field) — the "maybe absent" case is encoded in a union the caller must remember to narrow, with no combinators and no short-circuit. Boundary vs the Option SUBSTITUTION domain: that finder asks "did we reimplement an Option helper?"; this asks "is the *representation* itself dishonest?" — file as modelling.',
    nativeShape:
      'Option<T>: Option.fromNullable at the boundary, then map/flatMap/getOrElse over it so absence threads through composition instead of per-call null checks. NOTE: a `T | undefined` that is an external/library boundary type we cannot change, or a genuinely-optional config field that is never branched on as presence, is not this smell.',
    docs: 'data-types/option',
    nativeRef: 'Option, Option.fromNullable, Option.map/flatMap/getOrElse',
  },
  {
    key: 'sentinel-or-throw',
    grep: "a PURE / synchronous (non-Effect-returning) function that signals 'not found' / failure with a sentinel (-1, '', null, NaN, {}) OR by `throw`ing for an expected, recoverable case",
    smell:
      'A pure/sync function that encodes an expected failure or absence as a sentinel value (-1, empty string, null, NaN) or a `throw`, instead of a typed result the caller must handle. BOUNDARY vs structural throw-in-effect: that smell is a `throw` INSIDE an Effect-returning function; this one is the pure/sync complement — a sentinel or throw in a function that is NOT Effect-returning. If the function returns Effect, it is throw-in-effect, not this.',
    nativeShape:
      'Option<T> when the only failure is "absent"; Either<E, A> (or a Data.TaggedError carried in a later Effect E channel) when there is a meaningful error to report. The caller composes over the result instead of checking a sentinel or wrapping in try/catch.',
    docs: 'data-types/either',
    nativeRef: 'Option, Either, Either.left/right, Data.TaggedError',
  },
  {
    key: 'flag-stringly-state',
    grep: "state modelled as several booleans and/or optional fields on one object, or a bare string-literal `kind`/`type`/`status` checked ad hoc, where combinations of the fields can express illegal states",
    smell:
      'A state/variant modelled as a bag of booleans + optional fields (isLoading, isError, data?, error?) or an ad-hoc string tag, so illegal combinations (loading AND error, data present in the error case) are representable and must be defended by convention. Make-illegal-states-unrepresentable is discarded.',
    nativeShape:
      'A discriminated union — Data.taggedEnum (or a Schema tagged union) with one variant per legal state, each carrying exactly its own data — consumed with Match.exhaustive so the compiler proves every case is handled. NOTE: a single optional field with no cross-field invariant is not this smell; the smell needs ≥2 fields whose combinations include illegal states.',
    docs: 'data-types/data',
    nativeRef: 'Data.taggedEnum, Match.exhaustive, Schema tagged unions',
  },
  {
    key: 'bare-primitive',
    grep: "a domain value carried as a bare `string` / `number` (ids, tokens, urls, emails, slugs, counts with units) whose constraints are enforced ad hoc at use sites rather than encoded in its type",
    smell:
      'A domain concept with real constraints (a channel id, an auth token, a URL, a non-empty name, a positive count) typed as bare `string`/`number`, so nothing stops an arbitrary string flowing where a validated one is required, and validation is re-checked (or forgotten) at each use site. The type carries no guarantee.',
    nativeShape:
      'A branded / refined type: Schema.brand or Brand.nominal for a nominal distinction, Schema.NonEmptyString / Schema.pattern / Schema.URL / Schema.Int+positive for a refinement decoded once at the boundary, returning Effect<Brand, ParseError>. Interior code then receives the guarantee in its type. NOTE: a transient local string with no domain meaning is not this smell.',
    docs: 'code-style/branded-types',
    nativeRef: 'Schema.brand, Brand.nominal/refined, Schema.NonEmptyString/pattern/URL',
  },
  {
    key: 'in-place-mutation',
    grep: "building or transforming PURE data by mutation: array.push/splice/sort/reverse/fill/copyWithin in a loop, delete obj.k, or obj.k = v to accumulate a result that is then returned/used as a value",
    smell:
      'Pure data assembled by in-place mutation — push/splice/sort/reverse into an array, or property writes onto an object, to build a value that is then returned or compared. The data is not modelled as immutable, so aliasing and accidental later mutation are possible and value-equality is unsafe. BOUNDARY vs structural mutable-state: that smell is reassigned state CROSSING an Effect/async boundary; this is mutation of pure data within sync code, a representation choice. If the mutated value crosses an Effect boundary, it is mutable-state, not this.',
    nativeShape:
      'Immutable construction: build with Array map/filter/reduce / spread, or use a persistent collection (Chunk, HashMap, HashSet) and `readonly` types; for value objects use Data.struct/Data.array so the result is immutable AND gets structural equality. NOTE: a pure local accumulator built and consumed entirely within one synchronous function, never aliased or returned by reference, is acceptable — flag it only when the mutable value escapes or is used where immutability/equality matters.',
    docs: 'data-types/chunk',
    nativeRef: 'Array.map/filter/reduce, Chunk, HashMap, HashSet, Data.struct/array, readonly',
  },
  {
    key: 'hand-equality',
    grep: "hand-rolled value comparison: field-by-field a.x === b.x && a.y === b.y, JSON.stringify(a) === JSON.stringify(b), or dedup/membership via a Set/array keyed on a stringified value",
    smell:
      'Structural equality computed by hand — field-by-field comparison, JSON.stringify round-trips, or dedup/membership keyed on a stringified form — instead of declaring the type equatable. Brittle (field drift, key order, undefined vs absent) and not reusable. BOUNDARY vs the Equal SUBSTITUTION domain: that finder spots a reimplemented Equal/Hash helper; this spots a type whose REPRESENTATION should carry value-equality but does not — file as modelling.',
    nativeShape:
      'Make the type a value type with Data.struct / Data.case (or implement Equal + Hash), then use Equal.equals and the value-based HashSet/HashMap; equality and hashing come for free and stay correct as fields change. NOTE: comparing two genuinely-primitive scalars with === is fine — this is for structural/compound values.',
    docs: 'trait/equal',
    nativeRef: 'Equal.equals, Data.struct/case, Hash, HashSet/HashMap',
  },
  {
    key: 'data-first-helper',
    grep: "a project-defined helper used inside pipe(...) only via an arrow wrapper `(x) => helper(config, x)` because its own signature takes the data FIRST, breaking point-free composition in the pipe",
    smell:
      'A project helper whose signature puts the data (the thing flowing through the pipe) FIRST, so every pipe use must wrap it in `(x) => helper(cfg, x)`. The goal is clean, point-free pipe composition; a data-first signature defeats it and litters pipes with arrow wrappers.',
    nativeShape:
      'Give the helper a data-last (curried) signature — `helper(config) => (data) => result`, or a `dual` API (Effect ships `dual` for exactly this) — so it drops straight into pipe with no wrapper. GUARD AGAINST OVERREACH: this is in service of real pipe composability, NOT currying for its own sake. Do NOT propose data-last for a helper that is never used in a pipe, called with all args at one site, or where the data-last form would be gratuitous point-free that reads worse. The verify pass refutes those.',
    docs: 'code-style/dual',
    nativeRef: 'data-last convention, dual (Function.ts), pipe',
  },
]

// ---------------------------------------------------------------------------
// Behaviour smell catalogue — the FOURTH axis. AUTHORITATIVE copy; domains.md mirrors it.
// The other three axes are STATIC: they catch non-native code that is PRESENT (a rolled helper, an
// imperative spine, a dishonest type). Behaviour is DYNAMIC: native-but-misused / policy code — how
// effects RUN (concurrency), FAIL (the error channel), and what they TRUST (the validation boundary).
// `Effect.all` exists and is used — but unbounded, or sequentially when it could be concurrent; a
// `fork` exists — but unsupervised; a decode exists in the toolbox — but the edge skips it. Each finder
// anchors on PRESENT code (an Effect.all/fork, an `as`/JSON.parse, a typed E) — NOT pure-absence
// scanning, which is too noisy ("no timeout anywhere"). Grounded in the concurrency / error-management
// / schema design docs. Three smells border an existing axis; each states its boundary so they don't
// double-count (synthesis still dedups same file:line). `docs` is a docs slug, same as above.
// ---------------------------------------------------------------------------
const BEHAVIOUR_SMELLS = [
  {
    key: 'sequential-not-concurrent',
    grep: "a sequence of INDEPENDENT effects run one-after-another — successive `yield*` of effects that do not use each other's result, or Effect.forEach / Effect.all over independent work with the default (sequential) concurrency",
    smell:
      'Independent effects executed sequentially when they have no data dependency on each other — a chain of `yield* a; yield* b; yield* c` or a default-concurrency forEach — so total latency is the SUM of the calls instead of the max. The work is parallelisable and is not being parallelised.',
    nativeShape:
      'Effect.all([...], { concurrency }) or Effect.forEach(items, f, { concurrency }) with a sensible bound. THE KEY EXCLUSION: this is ONLY a smell when the effects are genuinely INDEPENDENT. If each effect consumes a prior effect\'s result, sequential is correct — do NOT flag it. Likewise if ordering or a rate limit makes sequencing intentional.',
    docs: 'concurrency/basic-concurrency',
    nativeRef: 'Effect.all { concurrency }, Effect.forEach { concurrency }',
  },
  {
    key: 'unbounded-fanout',
    grep: "Effect.all / Effect.forEach with concurrency:'unbounded', or a fork-per-item loop (Effect.fork inside a map/forEach), over a collection whose size is driven by external/unbounded input and that hits a shared external resource (a socket peer, a subprocess, a tracker, an HTTP API)",
    smell:
      'Concurrency with NO upper bound over a collection the program does not control — concurrency:"unbounded", or forking one fiber per item of an externally-sized list — fanning out onto a shared resource. Under a large input this is a self-inflicted rate-limit / resource exhaustion (connections, processes, file handles).',
    nativeShape:
      'A BOUNDED concurrency: Effect.all/forEach with { concurrency: <n> } (or a Semaphore / Effect.withConcurrency) sized to what the downstream tolerates. NOTE: unbounded is fine for a provably small, fixed collection, or work that touches no shared/contended resource — flag it only when the size is externally driven AND the target is shared.',
    docs: 'concurrency/basic-concurrency',
    nativeRef: 'Effect.all/forEach { concurrency: n }, Effect.Semaphore, Effect.withConcurrency',
  },
  {
    key: 'unsupervised-fork',
    grep: "Effect.fork / forkDaemon / forkScoped / forkIn / runFork whose Fiber is never joined or awaited and has no error handler, or whose lifetime does not match the work it serves",
    smell:
      'A forked fiber whose failure nothing observes (never joined or awaited, no forkWithErrorHandler), so it fails silently; or a fork whose lifetime does not match its job. A bare `Effect.fork` is auto-supervised: it is interrupted, finalizers included, when its PARENT FIBER ends. That is a bug only when the work must outlive the parent, such as a listener forked from a short-lived handler. forkDaemon is the opposite bug when the work should end with its owner.',
    nativeShape:
      'Observe the failure: Fiber.join / Fiber.await, Effect.forkWithErrorHandler, or Effect.all/race when the fork was only ever concurrency. Match the lifetime: keep bare Effect.fork when the parent fiber is the right owner, Effect.forkScoped / Effect.forkIn when a Scope owns it, Effect.forkDaemon only for genuinely global work. NOTE: a fork whose lifetime matches its job and whose failures are observed is fine, whichever variant it uses.',
    docs: 'concurrency/fibers',
    nativeRef: 'Effect.fork, Effect.forkScoped, Effect.forkIn, Effect.forkDaemon, Effect.forkWithErrorHandler, Fiber.join/await, Effect.all/race',
  },
  {
    key: 'unvalidated-boundary',
    grep: "untrusted external data entering the domain WITHOUT a schema decode: `as SomeType` / `as unknown as` on a fetch/SDK/JSON.parse result, or JSON.parse(...) whose result is used directly, or trusting a third-party client's return type as if validated",
    smell:
      'External, untrusted data — an HTTP response body, env value, message payload, third-party SDK return, or JSON.parse result — entering the domain via an `as` cast or by trusting the static type, with NO runtime validation. The compiler is told a shape it never verified; malformed input becomes a silent defect deep inside. BORDER: this is the ABSENCE of a decode (vs structural internal-bridge, which is a decodeUnknownSync bridge that DOES exist at a seam; vs modelling bare-primitive, which is a domain value lacking a brand). If a *Sync decode exists, that is internal-bridge; if a domain value just lacks a brand, that is bare-primitive; this is "no decode at all at the trust boundary".',
    nativeShape:
      'Schema.decodeUnknown(MySchema)(input) at the boundary, returning Effect<A, ParseError, never> — the untrusted value is parsed into a validated domain type once, at the edge, and the interior receives the guarantee. NOTE: data from a genuinely trusted/internal source already decoded upstream, or a cast to a type the program fully owns end-to-end, is not this smell.',
    docs: 'schema/getting-started',
    nativeRef: 'Schema.decodeUnknown, Schema.decode, ParseResult',
  },
  {
    key: 'untyped-error-channel',
    grep: "an Effect whose error channel E is `unknown`/`string`/`{}` rather than a tagged union; OR catchAll / catchAllCause / orDie / ignore / Effect.option that swallows or downgrades a recoverable, typed failure. (E = the global `Error`, and a tryPromise/try `catch` returning `Error` or `unknown`, already fail the typecheck as globalErrorInEffectFailure / globalErrorInEffectCatch / unknownInEffectCatch.)",
    smell:
      'The typed E channel is not designed: it is `Error` / `unknown` / a string (so callers cannot exhaustively handle it), or a recoverable typed failure is swallowed/downgraded — catchAll that discards the error, orDie turning a tracked failure into a defect, ignore/Effect.option dropping it — losing the very tracking the E channel exists for. BORDER vs structural throw-in-effect: that is a raw `throw` escaping into a defect; this is the SHAPE and HANDLING of the typed channel itself.',
    nativeShape:
      'Model failures as a tagged union of Data.TaggedError variants in E, and handle them with catchTag / catchTags / Match so each case is addressed (or deliberately re-failed) — not blanket-swallowed. Reserve orDie/die for genuine defects. NOTE: a deliberately broad E at a top-level convergence point that maps everything to one user-facing report, or a catchAll that genuinely HANDLES (recovers + continues) rather than swallows, is fine.',
    docs: 'error-management/two-error-types',
    nativeRef: 'Data.TaggedError, Effect.catchTag/catchTags, Effect.fail vs Effect.die',
  },
]

// ---------------------------------------------------------------------------
// What the plugin's two linters already own — finders must NOT re-report it.
// The typecheck is Effect's tsgo build of tsc, run on plugin/ by `nix flake check`; with the
// plugin's tsconfig every Effect diagnostic it emits fails the check, suggestions included. The
// rules listed are the default-on ones that border this sweep, measured against the effect-tsgo
// version flake.nix pins. The Biome GritQL plugin lints its own two patterns and leaves the
// judgment-heavy axes to this sweep. The global-API rules are off by default, so the sweep keeps them.
// ---------------------------------------------------------------------------
const LINT_OWNED = `Do NOT report anything the plugin's linters already report:
- The typecheck (Effect's language service, built into tsc): a floating Effect (floatingEffect),
  a bare yield in a generator (missingStarInYieldEffectGen), an Effect.gen holding one return
  (unnecessaryEffectGen), Effect.succeed(undefined) for Effect.void (effectSucceedWithVoid),
  Effect.map(() => undefined) for Effect.asVoid (effectMapVoid), returning an Effect from a
  generator (returnEffectInGen), a *Sync Schema decode inside a generator (schemaSyncInEffect),
  try/catch inside a generator (tryCatchInEffectGen), Effect.run* inside a generator
  (runEffectInsideEffect), catchAll + fail for mapError (catchAllToMapError), catchAll + succeed for
  orElseSucceed (catchToOrElseSucceed), catching an Effect that cannot fail (catchUnfailableEffect),
  Effect.all over Array#map for Effect.forEach (allOfMapToForEach), the global Error in E
  (globalErrorInEffectFailure), and a catch callback returning Error or unknown
  (globalErrorInEffectCatch, unknownInEffectCatch).
- The Biome GritQL plugin (plugin/biome-plugins/effect-native-predicates.grit): \`x instanceof Error\`
  for Predicate.isError, and a switch default holding a \`const _: never\` exhaustiveness guard for
  Match.value(...).pipe(Match.discriminatorsExhaustive(...)).
These are NOT linted here, so they ARE yours to report: Date.now/new Date, console, fetch,
setTimeout/setInterval, Math.random, process.env, JSON.parse/stringify, new Promise, async functions,
node: built-in imports, classes extending Error, and E typed unknown.
Your value is the duplication, shape, representation and policy a per-line linter is blind to.`

const FINDING_FIELDS = {
  file: { type: 'string', description: 'repo-relative path, e.g. plugin/src/watcher.ts' },
  line: { type: 'number', description: 'first line of the rolled-our-own block' },
  kind: { type: 'string', enum: ['native-replacement', 'idiom', 'structural', 'modelling', 'behaviour'] },
  rolled: { type: 'string', description: 'what our code does today — the rolled helper, or for kind=structural the current OOP/imperative shape, or for kind=modelling the current data/state representation, or for kind=behaviour the current concurrency/failure/trust behaviour' },
  native: { type: 'string', description: 'the Effect form it maps to — a helper like Schedule.exponential, or for kind=structural the Effect-native shape (make Effect, Tag+Layer, acquireRelease...), or for kind=modelling the honest representation (Option, Data.taggedEnum, branded type...), or for kind=behaviour the native dynamic form (Effect.all { concurrency }, forkScoped, Schema.decodeUnknown, tagged E)' },
  confidence: { type: 'string', enum: ['high', 'low'] },
  why: { type: 'string', description: 'docs-grounded justification that the swap/rewrite preserves behaviour' },
  sourceRef: { type: 'string', description: 'Effect source path + export, or for kind=structural/modelling/behaviour the native API the shape uses' },
  docsRef: { type: 'string', description: 'docs slug / page consulted (e.g. requirements-management, error-management/two-error-types, data-types/option, concurrency/basic-concurrency)' },
  smell: { type: 'string', description: 'kind=structural the structural catalogue key (oop-construction, manual-di, throw-in-effect, internal-bridge, mutable-state, imperative-lifecycle); kind=modelling the modelling catalogue key (nullable-return, sentinel-or-throw, flag-stringly-state, bare-primitive, in-place-mutation, hand-equality, data-first-helper); kind=behaviour the behaviour catalogue key (sequential-not-concurrent, unbounded-fanout, unsupervised-fork, unvalidated-boundary, untyped-error-channel)' },
  blastRadius: { type: 'string', description: 'kind=structural/modelling/behaviour ONLY: the call sites / files a rewrite ripples to — a representation, shape, or concurrency/error-channel change is rarely local; name them' },
}

const FINDINGS_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    domain: { type: 'string' },
    inventorySize: { type: 'number', description: 'count of exports surveyed for this module' },
    fellBackToFetch: { type: 'boolean', description: 'true if the local clone was absent and source was fetched from GitHub' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: FINDING_FIELDS,
        required: ['file', 'line', 'kind', 'rolled', 'native', 'confidence', 'why', 'sourceRef', 'docsRef'],
      },
    },
  },
  required: ['domain', 'inventorySize', 'fellBackToFetch', 'findings'],
}

const STRUCTURAL_FINDINGS_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    smell: { type: 'string', description: 'the structural smell key scanned for' },
    fellBackToFetch: { type: 'boolean', description: 'true if the local docs clone was absent and docs were fetched from effect.website' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: FINDING_FIELDS,
        required: ['file', 'line', 'kind', 'rolled', 'native', 'confidence', 'why', 'sourceRef', 'docsRef', 'smell', 'blastRadius'],
      },
    },
  },
  required: ['smell', 'fellBackToFetch', 'findings'],
}

const VERDICT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    refuted: { type: 'boolean', description: 'true if the proposed native swap is wrong, a false twin, or changes behaviour' },
    reason: { type: 'string', description: 'docs/source-grounded justification for the verdict' },
  },
  required: ['refuted', 'reason'],
}

// The agent owns only the report prose + the dedup it reasons about while writing it. The
// confirmed/lowConfidence partition is NOT agent-returned: an LLM asked to losslessly reproduce a
// split the script can compute from each finding's own confidence dropped and misfiled survivors.
// The gate derives those arrays deterministically via splitSurvivors.
const SYNTH_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    reportMarkdown: { type: 'string', description: 'the full report, also written to the report file' },
  },
  required: ['reportMarkdown'],
}

// Deterministic dedup of findings proposing the same remedy at one file:line (highest confidence
// wins), then the confidence split. A different axis, smell or replacement is different work.
function splitSurvivors(survivors) {
  const bySite = new Map()
  for (const finding of survivors) {
    const site = JSON.stringify([finding.file, finding.line, finding.axis, finding.smell ?? '', finding.native])
    const existing = bySite.get(site)
    if (!existing || (finding.confidence === 'high' && existing.confidence !== 'high')) {
      bySite.set(site, finding)
    }
  }
  const deduped = [...bySite.values()]
  return {
    confirmed: deduped.filter((f) => f.confidence === 'high'),
    lowConfidence: deduped.filter((f) => f.confidence !== 'high'),
  }
}

// Where a docs slug is read from: the local checkout when args.docsDir names one, else the tag on GitHub.
function docsLocation(slug) {
  if (DOCS_DIR) return `${DOCS_DIR}/${slug}`
  return `Effect-TS/website at tag ${DOCS_TAG}, path ${DOCS_PATH}/${slug} — read a page with
   curl -sfL https://raw.githubusercontent.com/Effect-TS/website/${DOCS_TAG}/${DOCS_PATH}/${slug}.mdx
   and list a section with
   gh api 'repos/Effect-TS/website/contents/${DOCS_PATH}/${slug}?ref=${DOCS_TAG}' --jq '.[].name'`
}

// Shared docs-grounding instruction. A slug resolves to a page (.mdx) or a section directory (read
// every .mdx inside). null = the module is API-reference only, no prose.
function docsInstruction(slug) {
  if (!slug) {
    return `This module is API-reference only — there is no prose docs page. Ground the when/why in
   the source inventory + JSDoc @example blocks.`
  }
  return `read the Effect 3 docs at ${docsLocation(slug)}
   — the design rationale (the *when & why*).
   That slug is EITHER a single page (${slug}.mdx) OR a section DIRECTORY: if it is a directory,
   read EVERY .mdx inside — those are the section's pages, and reading only the intro is NOT reading
   the docs (e.g. stream/ is creating + consuming + operations + error-handling + resourceful;
   requirements-management/ is services + layers + default-services + layer-memoization).
   Do NOT read effect.website or the website's main branch: they document Effect 4. Where the docs
   name an API the installed source lacks, the source wins. Set fellBackToFetch=true if you read
   the docs from GitHub. The docs tell you WHEN a tool is right and when it is NOT.`
}

// What a refuter reads its evidence from: the same pinned source and Effect 3 docs the finders read.
function verifierGrounding() {
  return `Read Effect's source from the installed packages, ${SOURCE}/<package>/src/. If ${SOURCE} is
absent, fetch the version plugin/package.json pins from https://unpkg.com/<package>@<version>/.
Read each docs slug the finding cites at ${docsLocation('<slug>')}.
Do NOT read effect.website, the website's main branch or Effect's main branch: they are Effect 4.
Where the docs and the installed source disagree, the source wins.`
}

function finderPrompt(d) {
  const srcList = d.src.map((s) => `${SOURCE}/${s}`).join('\n       ')
  return `You are auditing ONE Effect domain — **${d.key}** — for places this codebase hand-rolled
what Effect ships natively, and for non-idiomatic phrasing a per-line linter cannot catch.

WORK INVENTORY-FIRST. Do not look at our code until you hold the module's full vocabulary.

1. INVENTORY (the *what*): list the module's complete export surface from the installed source:
       ${srcList}
   grep -nE '^export ' each file, AND read every \`export { ... }\` block in full: Effect re-exports
   names there, often renamed (\`_void as void\`, \`URL$ as URL\`), so a grep for
   \`^export (const|function|...)\` alone misses Effect.void, Data.case, Schema.URL and
   Schema.decodeUnknownSync.
   ${SOURCE_FALLBACK}
   Read JSDoc + @example blocks for the non-obvious helpers so you know what each one does.

2. DOCS (the *when & why*): ${docsInstruction(d.docs)}
   This is what separates a real swap from a false twin (different laziness, error channel,
   ordering, or short-circuit semantics).

3. SCAN our code, now holding the full inventory:
   ${AUDIT_SCOPE}
   Flag blocks of logic that re-express any helper in the inventory (kind="native-replacement"),
   or that could be phrased more idiomatically with one (kind="idiom").

${LINT_OWNED}

Confidence: "high" only when the docs confirm the swap preserves behaviour exactly. If you are
unsure whether semantics match, mark "low" — the verify pass will scrutinise it.

Return your findings AND inventorySize (how many exports you surveyed). Empty findings is a valid,
honest result — do not invent matches to look productive.`
}

function refutePrompt(f) {
  return `Adversarially verify ONE proposed Effect-native swap. Your job is to REFUTE it.

  file:       ${f.file}:${f.line}
  rolled:     ${f.rolled}
  native:     ${f.native}
  claim/why:  ${f.why}
  source:     ${f.sourceRef}
  docs:       ${f.docsRef}

Read the actual code at ${f.file} around line ${f.line}, and check the Effect source/docs cited.
${verifierGrounding()}
Try hard to prove the swap is WRONG: a false twin (looks equivalent but differs in laziness,
error channel, ordering, short-circuit, or null/undefined handling), changes observable behaviour,
loses a needed edge case, or the native helper does not actually exist with that signature.

Default to refuted=true when you cannot confirm the swap is safe. Only refuted=false when you have
positively verified, against source and docs, that the native form preserves behaviour.`
}

function structuralFinderPrompt(s) {
  return `You are auditing this codebase for ONE Effect-native STRUCTURAL smell — **${s.key}**.

This axis is NOT about a missing helper. It is about program SHAPE: code where the leaves return
Effect but the construction / dependency-injection / error / resource SPINE is still imperative or
OOP. "Effect used inside functions" is NOT the same as "an Effect-native program" — read the
getting-started/the-effect-type page, from ${docsLocation('getting-started/the-effect-type')}
if you need the framing (effects are lazy *descriptions*, not eagerly-executed state-deriving
constructors).

THE SMELL:
  ${s.smell}

THE EFFECT-NATIVE SHAPE:
  ${s.nativeShape}
  Native API: ${s.nativeRef}

1. GROUND in the design docs (the *why*): ${docsInstruction(s.docs)}
   Understand WHY the native shape is preferred AND when the imperative shape is genuinely
   justified — you must be able to tell a real smell from a deliberate, correct choice.

2. SCAN our code for this shape:
   ${AUDIT_SCOPE}
   Anchor your search on: ${s.grep}
   A grep hit is NOT a finding until you have READ the site and confirmed the shape — and ruled
   out a legitimate reason for it (see the caveats in the native-shape note above).

${LINT_OWNED}

For each instance: kind="structural", smell="${s.key}", rolled=the current shape (what the code
does today, with the smell), native=the concrete Effect-native rewrite for THIS site, and
blastRadius=the call sites / files a rewrite would ripple to (structural rewrites are rarely local;
name them — that ripple is the real cost and the reviewer needs it).

Confidence "high" ONLY when the native shape clearly fits AND you see no justification for the
current shape. If there may be a real reason — per-instance parameterisation that forbids a
singleton, a genuine defect that should stay a throw, a pure local accumulator — mark "low"; the
verify pass will scrutinise it.

Empty findings is a valid, honest result. Do not invent matches to look productive.`
}

function refuteStructuralPrompt(f) {
  return `Verify ONE structural finding. The verdict question is narrow: is this a FALSE POSITIVE —
i.e. NOT actually a non-Effect-native shape worth surfacing? Refute (refuted=true) ONLY if so.

  file:          ${f.file}:${f.line}
  smell:         ${f.smell}
  currentShape:  ${f.rolled}
  nativeShape:   ${f.native}
  blastRadius:   ${f.blastRadius}
  claim/why:     ${f.why}
  docs:          ${f.docsRef}

Read the actual code at ${f.file} around line ${f.line} and the cited design docs.
${verifierGrounding()}

REFUTE (refuted=true) only when one of these holds — the smell is not real:
- The finder MISREAD the code — it is not that shape (e.g. the "class" is a Schema class, a
  Data.TaggedError, or a Context.Tag subclass — those are idiomatic, not a class-as-service; the
  "throw" is a genuine DEFECT for a violated invariant that SHOULD stay a defect; the "mutable
  state" is a pure local accumulator that never crosses an Effect boundary; the run* IS the single
  app-edge call).
- The current shape is genuinely JUSTIFIED and the docs support keeping it, AND the finding offers
  no correct native alternative.

Do NOT refute for any of these — they are concerns for the HUMAN GATE that reviews confirmed
findings, NOT grounds to discard a real smell:
- "Possible but maybe not worth it" / "adds ceremony" / "the wrapped construction is pure sync".
- "The blast radius is large" or "understated" — if the blast radius is wrong, say so in the
  reason and set blastRadius straight, but CONFIRM (refuted=false). The gate weighs cost.
- "I cannot be certain the whole multi-site refactor is correct" — you are not verifying a finished
  refactor; the exact target shape is the implementer's job. You are only judging
  whether the SMELL is real.
- The proposed nativeShape is imperfect but the smell and its direction are right — CONFIRM and note
  the correction in the reason.

Default to refuted=FALSE when unsure: a surfaced false positive is cheaply cut at the gate, but a
refuted true smell is invisible. Only refute when you can affirmatively show the smell is not real.`
}

function modellingFinderPrompt(s) {
  return `You are auditing this codebase for ONE Effect-native MODELLING smell — **${s.key}**.

This axis is NOT about a missing helper (substitution) or an imperative effect/DI/resource spine
(structural). It is about how DATA and STATE are REPRESENTED: a type or shape that throws away a
guarantee Effect's data types (Option, Either, Data tagged unions, branded/refined Schema types,
persistent collections, Equal) would give you for free. The dishonesty is in the TYPE, not in a
function call and not in the effect spine — the cited design doc below is your framing.

THE SMELL:
  ${s.smell}

THE EFFECT-NATIVE REPRESENTATION:
  ${s.nativeShape}
  Native API: ${s.nativeRef}

1. GROUND in the design docs (the *why*): ${docsInstruction(s.docs)}
   Understand WHY the honest representation wins AND when the plainer shape is genuinely fine — you
   must tell a real smell from a deliberate, correct choice (the NOTE in the representation above is
   your exclusion list; respect it).

2. SCAN our code for this representation:
   ${AUDIT_SCOPE}
   Anchor your search on: ${s.grep}
   A grep hit is NOT a finding until you have READ the site and confirmed the shape — and ruled out
   the documented legitimate cases.

${LINT_OWNED}

For each instance: kind="modelling", smell="${s.key}", rolled=the current representation (what the
type/shape is today), native=the concrete honest representation for THIS site, and blastRadius=the
call sites / files a representation change ripples to (a type change is rarely local — every
producer and consumer of the value moves; name them — that ripple is the real cost the gate weighs).

Confidence "high" ONLY when the honest representation clearly fits AND none of the documented
exclusions apply. If a plainer shape may be justified here, mark "low"; the verify pass scrutinises it.

Empty findings is a valid, honest result. Do not invent matches to look productive.`
}

function refuteModellingPrompt(f) {
  return `Verify ONE modelling finding. The verdict question is narrow: is this a FALSE POSITIVE —
i.e. is the current representation actually fine, or did the finder misread it? Refute
(refuted=true) ONLY if so.

  file:          ${f.file}:${f.line}
  smell:         ${f.smell}
  currentShape:  ${f.rolled}
  nativeShape:   ${f.native}
  blastRadius:   ${f.blastRadius}
  claim/why:     ${f.why}
  docs:          ${f.docsRef}

Read the actual code at ${f.file} around line ${f.line} and the cited design docs.
${verifierGrounding()}

REFUTE (refuted=true) only when one of these holds — the smell is not real:
- The finder MISREAD the code — e.g. the "T | undefined" is an unchangeable external/library boundary
  type; the "stringly state" is a single optional field with no cross-field invariant; the
  "bare primitive" is a transient local with no domain meaning; the "in-place mutation" is a pure
  local accumulator that never escapes the function (NOTE: if the mutated value crosses an Effect/async
  boundary it is the STRUCTURAL mutable-state smell, still real — confirm and note the reclassification);
  the "hand equality" compares genuine scalars with ===.
- For smell="data-first-helper" SPECIFICALLY: refute if the helper is never used inside a pipe, is
  always called with all arguments at one site, or the proposed data-last form would be gratuitous
  point-free that reads WORSE — currying for its own sake is not the goal; clean pipe composition is.
- The current representation is genuinely JUSTIFIED, the docs support keeping it, AND the finding
  offers no correct native alternative.

Do NOT refute for any of these — they are HUMAN-GATE concerns, NOT grounds to discard a real smell:
- "Possible but maybe not worth it" / "adds ceremony" / "only one call site today".
- "The blast radius is large / understated" — if wrong, correct it in the reason and set blastRadius
  straight, but CONFIRM (refuted=false). The gate weighs cost.
- "I cannot be certain the whole type migration is correct" — the exact target type is the
  implementer's job; you only judge whether the SMELL is real.
- The proposed nativeShape is imperfect but the smell and its direction are right — CONFIRM and note
  the correction in the reason.

Default to refuted=FALSE when unsure: a surfaced false positive is cheaply cut at the gate, but a
refuted true smell is invisible. Only refute when you can affirmatively show the smell is not real.`
}

function behaviourFinderPrompt(s) {
  return `You are auditing this codebase for ONE Effect-native BEHAVIOUR smell — **${s.key}**.

The other three axes are STATIC — they catch non-native code that is PRESENT (a rolled helper, an
imperative spine, a dishonest type). This axis is DYNAMIC: native-but-MISUSED or policy code — how
effects RUN (concurrency), FAIL (the error channel), and what they TRUST (the validation boundary).
The construct is usually already Effect-native (an Effect.all, a fork, a typed E); the smell is that
it is used with the wrong policy, or the edge skips a step it should take. Anchor on code that is
PRESENT — do NOT scan for blanket absence ("no timeout anywhere"); confirm a concrete misuse site.

THE SMELL:
  ${s.smell}

THE EFFECT-NATIVE BEHAVIOUR:
  ${s.nativeShape}
  Native API: ${s.nativeRef}

1. GROUND in the design docs (the *why*): ${docsInstruction(s.docs)}
   Understand WHY the native behaviour wins AND — critically for this axis — the EXCLUSIONS where the
   current behaviour is correct (genuine data dependency, a provably-bounded collection, a deliberate
   daemon, a trusted source, a deliberately-broad top-level error). The NOTE/EXCLUSION in the
   behaviour description above is your guard against over-flagging.

2. SCAN our code for this behaviour:
   ${AUDIT_SCOPE}
   Anchor your search on: ${s.grep}
   A grep hit is NOT a finding until you have READ the site, confirmed the misuse, AND ruled out the
   documented legitimate cases. For concurrency smells you MUST check the data dependency between the
   effects before flagging — that is the whole question.

${LINT_OWNED}

For each instance: kind="behaviour", smell="${s.key}", rolled=the current behaviour (what runs/fails/
trusts today), native=the concrete native behaviour for THIS site, and blastRadius=the call sites /
files a change ripples to (making work concurrent changes error semantics to parallel; changing the E
channel moves every caller's catch site; adding a decode changes the return type — name the ripple).

Confidence "high" ONLY when the misuse clearly holds AND none of the documented exclusions apply. If
the current behaviour may be the correct, deliberate choice, mark "low"; the verify pass scrutinises it.

Empty findings is a valid, honest result. Do not invent matches to look productive.`
}

function refuteBehaviourPrompt(f) {
  return `Verify ONE behaviour finding. The verdict question is narrow: is this a FALSE POSITIVE —
i.e. is the current concurrency / failure / trust behaviour actually correct, or did the finder
misread it? Refute (refuted=true) ONLY if so.

  file:          ${f.file}:${f.line}
  smell:         ${f.smell}
  currentShape:  ${f.rolled}
  nativeShape:   ${f.native}
  blastRadius:   ${f.blastRadius}
  claim/why:     ${f.why}
  docs:          ${f.docsRef}

Read the actual code at ${f.file} around line ${f.line} and the cited design docs.
${verifierGrounding()}

REFUTE (refuted=true) only when one of these holds — the smell is not real. The per-smell guards:
- sequential-not-concurrent: refute if the effects are genuinely DATA-DEPENDENT (each consumes a
  prior's result) and so MUST be sequential, or ordering/rate-limit makes sequencing intentional.
  This is THE common false positive — verify the dependency before confirming.
- unbounded-fanout: refute if the collection is provably small/fixed, or the concurrent work touches
  no shared/contended resource (no rate-limit hazard).
- unsupervised-fork: refute if the fiber's failure IS observed (joined, awaited, or an error handler)
  AND its lifetime matches its job. A bare Effect.fork is bound to its parent fiber and interrupted
  with it, so do not accept "unmanaged lifetime" for one unless the work must outlive that parent.
- unvalidated-boundary: refute if the data source is genuinely trusted/internal (already decoded
  upstream), the cast is to a type the program owns end-to-end, OR a *Sync decode actually exists
  (then it is the STRUCTURAL internal-bridge smell — note the reclassification, still real).
- untyped-error-channel: refute if the broad E is a deliberate top-level convergence to one report,
  or the catchAll genuinely HANDLES (recovers + continues) rather than swallows.

Do NOT refute for any of these — they are HUMAN-GATE concerns, NOT grounds to discard a real smell:
- "Possible but maybe not worth it" / "adds ceremony" / "the input is usually small in practice".
- "The blast radius is large / understated" — if wrong, correct it in the reason and set blastRadius
  straight, but CONFIRM (refuted=false). The gate weighs cost.
- "I cannot be certain the whole concurrency/error refactor is correct" — the exact target is the
  implementer's job; you only judge whether the SMELL is real.
- The proposed nativeShape is imperfect but the smell and its direction are right — CONFIRM and note
  the correction in the reason.

Default to refuted=FALSE when unsure: a surfaced false positive is cheaply cut at the gate, but a
refuted true smell is invisible. Only refute when you can affirmatively show the behaviour is correct.`
}

// ===========================================================================
const date = A.date || 'undated'
const reportPath = A.reportPath || `docs/effect-native-audit-${date}.md`
// All four axes run by default; A.axes (e.g. ['behaviour']) restricts the sweep — cheap to re-run
// a single axis after editing its prompts, or to resume just the one that failed.
const AXES = A.axes || ['substitution', 'structural', 'modelling', 'behaviour']

log(`Effect-native audit ${date}: axes=[${AXES.join(', ')}] — ${DOMAINS.length} substitution domains + ${STRUCTURAL_SMELLS.length} structural smells + ${MODELLING_SMELLS.length} modelling smells + ${BEHAVIOUR_SMELLS.length} behaviour smells over ${AUDIT_ROOTS.length} source roots`)

// Sweep -> Verify as a pipeline: a finder's findings flow straight into verification the moment
// that finder returns; no barrier between the phases. Verification picks its refuter by finding
// kind — false-twin scrutiny for helper swaps (default refuted), justified-shape scrutiny for
// structural / modelling / behaviour rewrites (default surfaced).
// A finder that throws (rate-limit / StructuredOutput miss) degrades to null rather than dropping
// its whole group off the pipeline — the group still shows in coverage, flagged failed, the
// obvious candidate to re-run on resume.
const verifyStage = (axis) => (result, item) => {
  if (!result) return { domain: item.key, axis, inventorySize: 0, fellBackToFetch: false, failed: true, verified: [] }
  const inventorySize = result.inventorySize ?? 0
  const fellBackToFetch = result.fellBackToFetch ?? false
  if (!result.findings.length) return { domain: item.key, axis, inventorySize, fellBackToFetch, failed: false, verified: [] }
  return parallel(
    result.findings.map((f) => () => {
      const prompt =
        f.kind === 'behaviour'
          ? refuteBehaviourPrompt(f)
          : f.kind === 'modelling'
            ? refuteModellingPrompt(f)
            : f.kind === 'structural'
              ? refuteStructuralPrompt(f)
              : refutePrompt(f)
      // A verifier that dies is no evidence either way, so its finding is unverified, not refuted.
      const unverified = (why) => ({ ...f, domain: item.key, axis, refuted: false, unverified: true, refuteReason: why })
      return agent(prompt, { label: `verify:${item.key}:${f.file}:${f.line}`, phase: 'Verify', schema: VERDICT_SCHEMA })
        .then((v) => (v ? { ...f, domain: item.key, axis, refuted: v.refuted, refuteReason: v.reason } : unverified('verifier returned no verdict')))
        .catch(() => unverified('verifier errored (rate-limit / StructuredOutput miss)'))
    }),
  ).then((verified) => ({ domain: item.key, axis, inventorySize, fellBackToFetch, failed: false, verified }))
}

// All four axes sweep concurrently — they share the agent concurrency cap, so launching them
// together just fills the same slot pool while preserving each axis's own no-barrier Sweep->Verify
// flow. Modelling and behaviour reuse STRUCTURAL_FINDINGS_SCHEMA — it carries the same `smell` +
// `blastRadius` fields a representation or dynamic-behaviour change needs.
const [sweptDomains, sweptStructural, sweptModelling, sweptBehaviour] = await Promise.all([
  AXES.includes('substitution')
    ? pipeline(
        DOMAINS,
        (d) => agent(finderPrompt(d), { label: `find:${d.key}`, phase: 'Sweep', schema: FINDINGS_SCHEMA }).catch(() => null),
        verifyStage('substitution'),
      )
    : Promise.resolve([]),
  AXES.includes('structural')
    ? pipeline(
        STRUCTURAL_SMELLS,
        (s) => agent(structuralFinderPrompt(s), { label: `shape:${s.key}`, phase: 'Sweep', schema: STRUCTURAL_FINDINGS_SCHEMA }).catch(() => null),
        verifyStage('structural'),
      )
    : Promise.resolve([]),
  AXES.includes('modelling')
    ? pipeline(
        MODELLING_SMELLS,
        (s) => agent(modellingFinderPrompt(s), { label: `model:${s.key}`, phase: 'Sweep', schema: STRUCTURAL_FINDINGS_SCHEMA }).catch(() => null),
        verifyStage('modelling'),
      )
    : Promise.resolve([]),
  AXES.includes('behaviour')
    ? pipeline(
        BEHAVIOUR_SMELLS,
        (s) => agent(behaviourFinderPrompt(s), { label: `behave:${s.key}`, phase: 'Sweep', schema: STRUCTURAL_FINDINGS_SCHEMA }).catch(() => null),
        verifyStage('behaviour'),
      )
    : Promise.resolve([]),
])

const domains = [...sweptDomains, ...sweptStructural, ...sweptModelling, ...sweptBehaviour].filter(Boolean)
const allVerified = domains.flatMap((d) => d.verified).filter(Boolean)
const survivors = allVerified.filter((f) => !f.refuted && !f.unverified)
const unverified = allVerified.filter((f) => f.unverified)
const refutedCount = allVerified.filter((f) => f.refuted).length
const coverage = domains.map((d) => ({ domain: d.domain, axis: d.axis, inventorySize: d.inventorySize, fellBackToFetch: d.fellBackToFetch, failed: !!d.failed, findings: d.verified.length, unverified: d.verified.filter((f) => f.unverified).length }))
const missedDomains = coverage.filter((c) => c.failed || c.unverified > 0).map((c) => c.domain)

log(`Verified: ${survivors.length} survivors, ${refutedCount} refuted, ${unverified.length} unverified across ${domains.length} groups${missedDomains.length ? `; ${missedDomains.length} groups incomplete (${missedDomains.join(', ')}) — re-run on resume` : ''}`)

phase('Synthesise')
let synthesis = null
let synthesisError = null
try {
  synthesis = await agent(
    `Synthesise the Effect-native audit for ${date}. You are given the findings that SURVIVED
adversarial verification (the refuted ones are already dropped). Coverage data is included so the
report can state what was surveyed.

Findings come from FOUR axes. kind="native-replacement"/"idiom" are SUBSTITUTION findings (a rolled
helper -> a native one). kind="structural" are SHAPE findings (an OOP/imperative spine -> an
Effect-native shape). kind="modelling" are REPRESENTATION findings (a dishonest data/state type ->
an honest one: Option, Data.taggedEnum, branded type, immutable structure, Equal, data-last helper).
kind="behaviour" are DYNAMIC findings (native-but-misused concurrency / failure / trust: sequential
work that could be concurrent, unbounded fan-out, unsupervised fork, unvalidated boundary, untyped/
swallowed error channel). Structural, modelling, and behaviour all carry \`smell\` and \`blastRadius\`
and are larger refactors than a swap.

1. DEDUP: the same file:line may be flagged by more than one finder, across axes. Merge duplicates,
   keeping the clearest mapping and citing every finder/smell that found it. Do NOT collapse findings
   from different axes into one — they propose different work (a helper swap vs a shape rewrite vs a
   representation change vs a behaviour/policy change). Watch the deliberate axis borders: modelling
   sentinel-or-throw vs structural throw-in-effect; modelling in-place-mutation vs structural
   mutable-state; modelling hand-equality / nullable-return vs the Equal / Option substitution domains;
   behaviour unvalidated-boundary vs structural internal-bridge vs modelling bare-primitive; behaviour
   untyped-error-channel vs structural throw-in-effect; behaviour unsupervised-fork vs structural
   mutable-state. If two such findings are the SAME site, keep the one whose axis matches the actual
   fix and note the other; if genuinely distinct work, keep both.
2. SPLIT for the report: confidence==="high" -> the Confirmed sections; everything else -> the
   Low-confidence section. After dedup, a line that is high in any finder is high. (The gate also
   computes the confirmed/low-confidence partition itself, deterministically from each survivor's
   confidence — you do NOT return those arrays; you own the report prose.)
3. Write the report to ${reportPath} (use the Write tool). Structure:
     ## Effect-native audit — ${date}
     ### Confirmed — substitution   <- table: file:line | rolled -> native | why | docsRef
     ### Confirmed — structural     <- table: file:line | smell | currentShape -> nativeShape | blastRadius | why
     ### Confirmed — modelling      <- table: file:line | smell | currentRepr -> nativeRepr | blastRadius | why
     ### Confirmed — behaviour      <- table: file:line | smell | currentBehaviour -> nativeBehaviour | blastRadius | why
     ### Low-confidence (report only)   <- all axes, noting which
     ### Unverified                 <- findings whose verifier died: file:line | axis | smell | rolled -> native
     ### Coverage                   <- finders run per axis, per-module inventorySize, any fellBackToFetch, any failed finder
4. Return reportMarkdown (identical to the file). That is all — the gate derives the
   confirmed/low-confidence lists from the survivors itself.

SURVIVORS (JSON):
${JSON.stringify(survivors, null, 2)}

UNVERIFIED (JSON) — no verifier ruled on these; list them, never fold them into Confirmed:
${JSON.stringify(unverified, null, 2)}

COVERAGE (JSON):
${JSON.stringify(coverage, null, 2)}`,
    { label: 'synthesise', phase: 'Synthesise', schema: SYNTH_SCHEMA },
  )
} catch (e) {
  synthesisError = String((e && e.message) || e)
  log(`Synthesis agent failed (${synthesisError}) — returning ${survivors.length} raw survivors for the orchestrator to write up`)
}

// The confirmed/low-confidence partition is derived from the survivors deterministically, NOT from
// the synthesis agent — an LLM asked to re-split by confidence drops and misfiles findings. The
// agent only writes the report prose; whether it succeeded or died, the gate owns this split.
const { confirmed, lowConfidence } = splitSurvivors(survivors)

log(
  synthesis
    ? `Report written to ${reportPath}: ${confirmed.length} confirmed, ${lowConfidence.length} low-confidence (split deterministically from ${survivors.length} survivors)`
    : `No report file written (synthesis failed); ${confirmed.length} confirmed / ${lowConfidence.length} low-confidence split from ${survivors.length} survivors for write-up`,
)

return {
  date,
  reportPath,
  reportWritten: !!synthesis,
  synthesisError,
  confirmed,
  lowConfidence,
  survivors,
  unverified,
  refutedCount,
  coverage,
  missedDomains,
}
