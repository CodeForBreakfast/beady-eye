---
name: effect-model-audit
description: Use when asked to audit the type modelling of beady-eye's Claude Code plugin — whether its domain types are honest, whether they can represent illegal states, or for a periodic model-level type review. Triggers on "audit our type modelling", "are our types honest", "can our types represent illegal states", "model-level type review", "audit our domain types". The judgement sibling to effect-native-audit: a workflow-driven reasoning sweep over the plugin's domain types, not an anchored regression net and not the Effect language service.
---

# Effect-model type audit

This skill sweeps the domain types of the plugin under `plugin/` for
model-level dishonesty. A type is dishonest when its shape lets a caller build
a value the domain does not mean, or forces a caller to fake one. The question
is whether the type's value space matches the domain's, and whether all its
producers and consumers mean the same thing by it.

Four lenses ask about the value space, each against each family of types:

- **L1 Illegal-representable.** Can the type represent a value that is invalid
  in the domain? A combination of fields, an optional that should be required,
  a bare primitive admitting an out-of-domain string, or a pair of booleans
  with a meaningless fourth corner. Invalid state must be unrepresentable.
- **L2 Under-expressive.** Is there a legal domain state the type cannot
  represent? The missing case shows up as a sentinel, a placeholder, an
  impossible branch, or a comment apologising for a field that is "always
  absent here".
- **L3 Role coherence.** Do all the type's construction sites and read sites
  mean the same thing by it, or does it serve two roles under one name? An
  address a caller supplies and an observation the outside world hands back
  are the usual pair. The tell is a producer that can legally build a value
  that is illegal for some consumer's role.
- **L4 Boundary honesty.** At a trust boundary, does the type's role demand a
  parsed-once strong representation that it lacks? Examples are a
  `Schema.Struct` with required fields, branded members, or a refinement
  decoded once. Parse at boundaries, trust inside.

## What it catches that the other checks cannot

Three checks cover the plugin's Effect code, and none re-reports another's
findings:

| Check | Driven by | Cost | Catches |
|---|---|---|---|
| Effect language service and the Biome rules in `plugin/biome-plugins/` | a rule per line, in the plugin typecheck and lint | free, every `nix flake check` | what one line shows, such as a floating Effect or `instanceof Error` |
| `effect-native-audit` | a `grep` pattern per finder | cheap, frequent | hand-rolled helpers, imperative spines, representation smells a pattern can find |
| `effect-model-audit` | a type family and its census, reasoned about | expensive, periodic | dishonesty with no lexical anchor, such as one type serving two roles |

## How it works

`effect-model-audit.workflow.js` runs four phases.

1. **Discover.** One agent runs a `grep` that lists every top-level type
   declaration in `plugin/src` and `plugin/hooks`, exported or not, and
   returns the names. Tests and the shared test helper `test-watcher.ts` are
   left out. A Schema constant counts as the type it decodes to. A second agent
   groups those names into families and gives each family its census: every
   site that builds or decodes a member, and every site that reads one.

   The families must partition the enumerated names exactly, and
   `assertPartition` checks that. Because the list comes from one agent and
   the grouping from another, the check compares the grouping with a list its
   author did not write. If the check fails, the workflow repairs the
   partition and puts anything left over into an `unassigned` family, so no
   type escapes the audit.
2. **Reason.** One agent per family and lens reads the type definitions and the
   census sites and reasons about the value space. No findings is a valid
   result.
3. **Verify.** Each finding goes to one adversarial refuter as soon as its
   finder returns. The refuter cuts any finding without the evidence its lens
   demands:
   - L1 needs a named illegal value.
   - L2 needs a named missing legal state and the workaround it forces.
   - L3 needs a disagreeing producer and consumer, both sites named.
   - L4 needs the specific boundary.

   Whether a remodelling is worth its blast radius is never grounds for
   refuting it. A verifier that errors keeps the finding, marked unverified.
4. **Synthesise.** An agent writes the report, merging findings that describe
   the same defect. L1 and L3 overlap by design. The confirmed and
   low-confidence lists the workflow returns come from `splitSurvivors`, not
   from the agent. It groups the findings at each `file:line` without dropping
   any, and confirms a site when any finding there is high-confidence.

Each surviving finding carries its evidence, a proposed remodelling, and its
blast radius: every producer and consumer the remodelling would touch.

## Running it

1. Install the plugin's dependencies if `plugin/node_modules` is absent, so
   agents can check an API against the pinned Effect:

   ```
   cd plugin && bun install --frozen-lockfile
   ```

2. Get today's date with `date +%F`.
3. Launch the workflow with the date:

   ```
   Workflow({
     scriptPath: '.claude/skills/effect-model-audit/effect-model-audit.workflow.js',
     args: { date: '<YYYY-MM-DD>' },
   })
   ```

   It runs in the background, and you are notified when it finishes. To run
   one lens alone, pass `args: { date, lenses: ['L3'] }`.

4. The workflow writes its report to `target/effect-model-audit-<date>.md`,
   which git ignores, and returns `{ confirmed[], lowConfidence[], survivors[],
   refutedCount, coverage, partitionOk }`.

The workflow reports findings and changes no code.

## Notes

- **`domains.md` holds the lens catalogue,** the modelling principles the
  lenses rest on, and where L4 ends and effect-native-audit begins.
- **The pinned Effect is the authority on its API.** `plugin/package.json` pins
  Effect 3. An agent names only an API that the installed copy under
  `plugin/node_modules/effect` exports.
- **The `LENSES` array in the workflow is the authoritative lens catalogue,**
  because the workflow cannot read files. `domains.md` mirrors it for people,
  so change the two together.
