# Lens catalogue and partition

This page mirrors the sweep's cell, a family of types crossed with a lens, for
people. The authoritative copy is the `LENSES` array in
`effect-model-audit.workflow.js`, because the workflow cannot read files. Keep
the two in sync.

## The partition

A **family** is a set of the plugin's domain types that share a role or refer
to one another, with its **census**: every site that builds or decodes a
member, and every site that reads one. A producer that can build a value
illegal for some consumer's role is invisible until the two sit side by side,
so the census is both the evidence for L3 and the blast radius of a
remodelling.

The domain types are every top-level `type`, `interface`, `class` and Schema
constant in `plugin/src/*.ts` and `plugin/hooks/*.ts`, exported or not. Tests
and the shared test helper `plugin/src/test-watcher.ts` are left out. The
plugin keeps most of its Schemas module-private, so a list of exports alone
would miss them.

Every enumerated type lands in exactly one family. `assertPartition` checks
the families against the enumerated list and fails on any of these:

- **missing**: a type in no family.
- **duplicated**: a type in more than one family.
- **unexpected**: a family member that was never enumerated.

One agent enumerates and another groups, so the check is against a list the
grouping did not write. On a failure the workflow drops members that were never
enumerated, keeps each type in its first family, and puts what is left into an
`unassigned` family.

## The principles

The lenses rest on modelling principles, with Effect's data types as the
vocabulary for the fix:

- Make invalid states unrepresentable.
- Strengthen, never weaken.
- Parse at boundaries, trust inside.
- A finite set of values is a union of literals.
- A value with a domain constraint is a brand.
- A field present in some states and absent in others calls for a
  discriminated union.

The plugin pins Effect 3. Name only an API that `plugin/node_modules/effect`
exports. These effect.website pages give the reasoning behind each data type:

- [`data-types/option`](https://effect.website/docs/data-types/option/)
- [`data-types/either`](https://effect.website/docs/data-types/either/)
- [`data-types/data`](https://effect.website/docs/data-types/data/)
- [`code-style/branded-types`](https://effect.website/docs/code-style/branded-types/)
- [`code-style/pattern-matching`](https://effect.website/docs/code-style/pattern-matching/)
- [`schema/introduction`](https://effect.website/docs/schema/introduction/)

## The four lenses

| Lens | Question | Honest representation | Principle |
|---|---|---|---|
| **L1** Illegal-representable | Can this type represent a value that is invalid in the domain? | narrow the type until its values are the domain's: required over optional, a branded or filtered Schema over a bare primitive, `Data.taggedEnum` over a pair of booleans | invalid states unrepresentable; `data-types/data` |
| **L2** Under-expressive | Is there a legal domain state this type cannot represent? | widen honestly: add the missing variant to a union, use `Option` where a field is genuinely optional, use a tagged case instead of a sentinel | finite set as a union; `data-types/option`, `data-types/either` |
| **L3** Role coherence | Do all producers and consumers mean the same thing, or is it two roles under one name? | split the roles: an address type apart from an observation type, a request apart from a response, a trusted form apart from an untrusted one | parse at boundaries |
| **L4** Boundary honesty | At a trust boundary, does the type's role demand a parsed-once strong type it lacks? | a `Schema.Struct` with required fields, branded members or a refinement, which the boundary decodes once and which then flows unchecked | parse at boundaries, trust inside; `schema/introduction` |

### The evidence each lens demands

A finding without its evidence is taste, and the verify pass refutes it.

- **L1** needs a named illegal value the type admits, as a concrete field
  assignment, and why the domain forbids it.
- **L2** needs the named missing legal state and the workaround it forces at a
  concrete site: a sentinel, a dummy in a required field, or an impossible
  branch.
- **L3** needs a disagreeing producer and consumer, with both sites named, and
  the concrete value one makes that the other cannot accept.
- **L4** needs the specific boundary and why its role demands the stronger
  type: which required fields, brands or refinement, and what defends the gap
  today.

Whether a remodelling is worth its blast radius is never grounds for refuting
a finding. A real illegal state stays a finding however far it ripples.

## Where lenses overlap

- **L1 and L3 overlap by design.** The same defect can be reached by reasoning
  about one type alone, which is L1, or by walking its census, which is L3.
  Neither suppresses a finding because the other might see it. Synthesis merges
  them by `file:line`. L3 runs even when L1 found nothing, because a type can be
  honest field by field and dishonest across its census.
- **L4 borders two other checks.** effect-native-audit's
  `unvalidated-boundary` smell is a missing `Schema.decodeUnknown` at a trust
  edge. The language service's `schemaSyncInEffect` rule is a `*Sync` decode
  inside an Effect. L4 is about the shape the type's role demands, whatever
  decode calls exist. A finding that is really one of those two belongs to its
  own check.

## Out of scope

- Anything the Effect language service reports in the plugin typecheck, and
  anything effect-native-audit finds from a pattern.
- Test files and `node_modules`.

## Refreshing

1. When the plugin's files move, check that `SURFACE`, `EXCLUDED` and the
   enumeration command in the workflow still name its domain types.
2. When the plugin moves to a new Effect major, check every API the lenses and
   the vocabulary name against the new version, and the effect.website pages
   above.
3. Add a lens only for a new value-space question that reaches a different
   defect by a different path, and edit the `LENSES` array and this table
   together.
