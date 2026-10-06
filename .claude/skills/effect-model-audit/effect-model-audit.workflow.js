export const meta = {
  name: 'effect-model-audit',
  description:
    "Model-level type audit of beady-eye's Claude Code plugin: whether its domain types can hold values the domain forbids, cannot hold values it allows, serve two roles under one name, or are loose where a trust boundary demands a parsed type. An enumeration pass lists every domain type in plugin/src and plugin/hooks, a grouping pass partitions them into families with their producer/consumer census, and the partition is checked against the enumeration. One agent then reasons over each family through each of four lenses, one refuter checks each finding against the evidence its lens demands, and a synthesis agent writes the report.",
  phases: [
    { title: 'Discover', detail: 'enumerate the domain types, group them into families with their census, check the partition' },
    { title: 'Reason', detail: 'one agent per family × lens: L1 illegal-representable, L2 under-expressive, L3 role coherence, L4 boundary honesty' },
    { title: 'Verify', detail: 'one refuter per finding, cutting any finding without the evidence its lens demands' },
    { title: 'Synthesise', detail: 'merge findings of one defect, group by site, split by confidence, write the report' },
  ],
}

// ---------------------------------------------------------------------------
// What is audited. The plugin keeps most of its domain types module-private,
// Schemas included, so the surface is every top-level type declaration in these
// files, exported or not. The census spans the same files, since every producer
// and consumer of a plugin type lives in the plugin.
// ---------------------------------------------------------------------------
const SURFACE = ['plugin/src/*.ts', 'plugin/hooks/*.ts']
const EXCLUDED = ['*.test.ts', 'plugin/src/test-watcher.ts']
const ENUMERATE_COMMAND = `grep -nE '^(export )?(type|interface|class) [A-Z]|^(export )?const [A-Z][A-Za-z0-9]* = Schema\\.' ${SURFACE.join(' ')} | grep -v -e '\\.test\\.ts:' -e 'test-watcher\\.ts:'`
const AUDIT_SCOPE = `The domain surface under audit is every top-level type declaration, exported or not, in:
  ${SURFACE.join('\n  ')}
excluding ${EXCLUDED.join(' and ')}. A Schema constant counts as the type it decodes to.
The producer/consumer census spans the same files. Report every finding by its repo-relative path,
for example plugin/src/watches.ts.`

// The modelling principles the lenses are grounded in, and where the vocabulary
// for an honest representation comes from. Effect is pinned in plugin/package.json;
// the installed copy under plugin/node_modules/effect is the authority on what
// that version provides.
const PRINCIPLES = `- Make invalid states unrepresentable: a type's admitted values should be exactly the domain's.
- Strengthen, never weaken: narrow a type to fit the domain rather than widen it to fit a caller.
- Parse at boundaries, trust inside: a trust boundary produces a strong type that then flows unchecked.
- A finite set of values is a union of literals, not a string.
- A value with a domain constraint is a brand, not a bare primitive.
- A field present in some states and absent in others is a discriminated union, not an optional.`
const VOCABULARY = `Effect's data types are the vocabulary for the honest representation: Option, Either,
Data.taggedEnum, Data.TaggedError, Schema.Struct with required fields, Schema.brand, Schema.filter,
Schema.TaggedStruct, Schema.Union of Schema.Literal. The plugin pins Effect 3 in plugin/package.json.
Name only an API that the installed copy under plugin/node_modules/effect exports (run \`bun install\`
in plugin/ first if it is absent), and propose nothing from another major version. For the reasoning
behind a data type, read its page under https://effect.website/docs/ — data-types/option,
data-types/either, data-types/data, code-style/branded-types, code-style/pattern-matching,
schema/introduction.`

// ---------------------------------------------------------------------------
// LENSES — the authoritative lens catalogue. domains.md mirrors it for people;
// keep the two in sync. Each lens is a question to reason about, not a pattern
// to match.
// ---------------------------------------------------------------------------
const LENSES = [
  {
    id: 'L1',
    title: 'Illegal-representable',
    question: 'Can this type represent a value that is invalid in the domain?',
    prompt: `Enumerate the set of values this type admits, then compare it with the set the domain
permits. If the type's set is larger — a combination of fields, an optional that should be required,
a bare primitive that admits an out-of-domain string, a pair of booleans whose fourth corner is
meaningless — the type can hold a value the domain forbids, and something downstream must defend that
corner by convention. Reason about the value space; do not match a token. EVIDENCE REQUIRED: name a
specific illegal value the type admits, as a concrete field assignment, and say why the domain forbids
it. "I would model this differently" with no named illegal value is taste, not a finding.`,
  },
  {
    id: 'L2',
    title: 'Under-expressive',
    question: 'Is there a legal domain state this type cannot represent?',
    prompt: `The dual of L1. Is there a state that is valid in the domain but that this type cannot
express, forcing a workaround: a sentinel value, a placeholder, an impossible branch the code handles
anyway, a comment apologising for a field that is "always absent here", a required field filled with a
dummy, an Option.none() standing in for "not that kind of thing"? EVIDENCE REQUIRED: name the specific
legal domain state the type cannot represent and the workaround it forces at a concrete site. Without
both there is no finding.`,
  },
  {
    id: 'L3',
    title: 'Role coherence',
    question: 'Do all producers and consumers mean the same thing by this type, or is it two roles under one name?',
    prompt: `Use the family's producer/consumer census. Walk every construction site and every read
site and ask whether they all mean the same thing by this type, or whether it serves two different
roles under one name: an address a caller supplies to target something versus an observation the
outside world hands back; a request versus a response; a trusted assembled form versus an untrusted
inbound one. The tell is a producer that can legally build a value which is illegal for some
consumer's role: an optional field one consumer must always have, or a field one producer fills that
another role must leave empty. Each field can be honest on its own while the type is dishonest across
its census, which is exactly what reasoning about one type in isolation misses. Run this lens even when
L1 found nothing. EVIDENCE REQUIRED: a specific disagreeing producer/consumer pair, naming both sites,
and the concrete value one makes that the other cannot accept.`,
    boundary: `Overlaps L1 by design: the same defect can be reached by reasoning about the type alone
(L1) or by walking its census (L3). Do not suppress an L3 finding because L1 might also see it, nor an
L1 finding because L3 is the census lens. Surface both; synthesis merges findings of the same defect.`,
  },
  {
    id: 'L4',
    title: 'Boundary honesty',
    question: "At a trust boundary, does the type's role demand a parsed-once strong type it does not have?",
    prompt: `At a trust boundary — anything the plugin reads from outside its own process — judge
whether this type's role demands a parsed-once strong representation (a Schema.Struct with required
fields, branded members, a decoded-once refinement) but is instead a loose shape the code defends by
convention: a bare interface where a decoded Struct belongs, unbranded primitives carrying domain
constraints, an index signature or optional-heavy fields standing in for "we'll check later". The
boundary should produce a strong type that then flows unchecked. Judge the type's role, not whether a
decode call happens to be present at some line. EVIDENCE REQUIRED: name the specific boundary and why
the role there demands the stronger type: which required fields, brands or refinement, and what defends
the gap today.`,
    boundary: `Borders effect-native-audit's unvalidated-boundary (a missing Schema.decodeUnknown at a
trust edge) and the language service's schemaSyncInEffect (a *Sync decode inside an Effect). L4 is
model-level: the type's role demands a strong parsed representation whether or not any decode call
exists. If the finding is really "a decode call is missing here" or "a *Sync decode runs inside an
Effect here", it belongs to one of those, not to L4. State the type-role reason to stay on this side of
the border.`,
  },
]
// --- end LENSES ---

// The coverage equation. Every enumerated type lands in exactly one family:
// missing is a type in no family, duplicated a type in more than one, and
// unexpected a family member that was never enumerated.
function assertPartition(types, units) {
  const enumerated = new Set(types)
  const seenCount = new Map()
  const unexpected = []
  for (const unit of units) {
    for (const member of unit.members) {
      if (!enumerated.has(member)) {
        if (!unexpected.includes(member)) unexpected.push(member)
        continue
      }
      seenCount.set(member, (seenCount.get(member) ?? 0) + 1)
    }
  }
  const missing = types.filter((t) => !seenCount.has(t))
  const duplicated = types.filter((t) => (seenCount.get(t) ?? 0) > 1)
  return {
    ok: missing.length === 0 && duplicated.length === 0 && unexpected.length === 0,
    missing,
    duplicated,
    unexpected,
  }
}

// The confirmed/low-confidence split is computed here rather than asked of an
// agent, which drops and misfiles findings when asked to reproduce a split.
// Findings at one file:line are grouped, not deduplicated: L1 and L3 can reach
// the same defect there, but two different defects can share a line too, and
// only a reader can tell which. A site is confirmed when any finding there is high.
function splitSurvivors(survivors) {
  const bySite = new Map()
  for (const finding of survivors) {
    const site = `${finding.file}:${finding.line}`
    const group = bySite.get(site) ?? { file: finding.file, line: finding.line, confidence: 'low', findings: [] }
    group.findings.push(finding)
    if (finding.confidence === 'high') group.confidence = 'high'
    bySite.set(site, group)
  }
  const sites = [...bySite.values()]
  return {
    confirmed: sites.filter((s) => s.confidence === 'high'),
    lowConfidence: sites.filter((s) => s.confidence !== 'high'),
  }
}

// ---------------------------------------------------------------------------
// Schemas.
// ---------------------------------------------------------------------------
const ENUMERATION_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    types: {
      type: 'array',
      items: { type: 'string' },
      description: 'every declared type name the command printed, once each, with no additions',
    },
  },
  required: ['types'],
}

const GROUPING_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    units: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: {
          key: { type: 'string', description: 'stable family name in kebab-case' },
          members: { type: 'array', items: { type: 'string' }, description: 'the type names in this family; the families must partition the given list exactly' },
          rationale: { type: 'string', description: 'why these types form one family: a shared role, a shared construction site, or mutual reference' },
          producers: { type: 'array', items: { type: 'string' }, description: 'every construction site, as file:line, where a member is built or decoded' },
          consumers: { type: 'array', items: { type: 'string' }, description: 'every read site, as file:line, where a member is destructured or consumed' },
        },
        required: ['key', 'members', 'rationale', 'producers', 'consumers'],
      },
    },
  },
  required: ['units'],
}

const LENS_FINDING_FIELDS = {
  unit: { type: 'string', description: 'the family key this finding belongs to' },
  lens: { type: 'string', enum: ['L1', 'L2', 'L3', 'L4'], description: 'the lens that surfaced it' },
  file: { type: 'string', description: 'repo-relative path of the type definition, e.g. plugin/src/watches.ts' },
  line: { type: 'number', description: 'the line of the field or member at fault, or the first line of the type when the fault is its whole shape' },
  evidence: {
    type: 'string',
    description: 'the concrete evidence — L1: a named illegal value the type admits; L2: a named missing legal state and the workaround it forces; L3: a disagreeing producer/consumer pair (both sites) and the value one makes the other cannot accept; L4: the specific boundary and why its role demands the stronger type',
  },
  remodelling: { type: 'string', description: 'the proposed honest representation for this type, using only APIs the pinned Effect version exports' },
  blastRadius: { type: 'string', description: 'every producer and consumer the remodelling ripples to, from the census' },
  confidence: { type: 'string', enum: ['high', 'low'], description: 'high only when the evidence is concrete and no plainer shape is justified' },
  why: { type: 'string', description: 'the modelling principle the finding rests on' },
}

const LENS_FINDINGS_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    unit: { type: 'string' },
    lens: { type: 'string' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        properties: LENS_FINDING_FIELDS,
        required: ['unit', 'lens', 'file', 'line', 'evidence', 'remodelling', 'blastRadius', 'confidence', 'why'],
      },
    },
  },
  required: ['unit', 'lens', 'findings'],
}

const VERDICT_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    refuted: { type: 'boolean', description: 'true if the finding lacks the evidence its lens demands, misreads the code, or the current model is correct' },
    reason: { type: 'string', description: 'the evidence or principle behind the verdict' },
  },
  required: ['refuted', 'reason'],
}

const SYNTH_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  properties: {
    reportMarkdown: { type: 'string', description: 'the full report, also written to the report file' },
  },
  required: ['reportMarkdown'],
}

// ---------------------------------------------------------------------------
// Prompts.
// ---------------------------------------------------------------------------
function enumerationPrompt() {
  return `Run this command from the repository root and report what it lists. Do not read any file
and do not add or drop a name on judgement.

  ${ENUMERATE_COMMAND}

Each line declares one type. Return the declared name from each line, once each: a Schema constant and
the type alias of the same name are one name. Drop any generic parameters, so \`Settings<R>\` is
\`Settings\`.`
}

function groupingPrompt(types) {
  return `You are the grouping pass of a model-level type audit. Group the plugin's domain types into
type families, each bundled with its full producer/consumer census. Reasoning about a type without its
usage misses defects that only show across producers and consumers, so the census is the heart of this
pass.

THE TYPES (exactly these, enumerated from the source):
  ${types.join(', ')}

1. GROUP them into families. A family is a set of types that share a role or refer to one another.
   Every type above lands in exactly one family, and no family names a type that is not above. This
   partition is checked mechanically after you return.

2. CENSUS each family. For every member, find every construction site (where it is built or decoded:
   producers) and every read site (where it is destructured or consumed: consumers). Record each as
   file:line. The census is the blast radius of a remodelling and the evidence for the role-coherence
   lens.

${AUDIT_SCOPE}

Completeness matters more than tidiness.`
}

function lensFinderPrompt(unit, lens) {
  return `You are auditing one type family, **${unit.key}**, through one lens, **${lens.id} ${lens.title}**.
You hold the family's producer/consumer census. Reason about the value space; do not scan for a token.

THE FAMILY:
  members:   ${(unit.members || []).join(', ')}
  rationale: ${unit.rationale || '(none given)'}
  producers: ${(unit.producers || []).join(', ') || '(none found)'}
  consumers: ${(unit.consumers || []).join(', ') || '(none found)'}

THE LENS — ${lens.id} ${lens.title}. Question: ${lens.question}
${lens.prompt}
${lens.boundary ? `\nBOUNDARY (do not double-count):\n${lens.boundary}` : ''}

GROUND your reasoning in these modelling principles:
${PRINCIPLES}

${VOCABULARY}

READ the actual type definitions and the census sites.
${AUDIT_SCOPE}

For each finding give unit="${unit.key}", lens="${lens.id}", the file:line of the field or type at fault, the
evidence this lens demands (a finding without it will be refuted), the remodelling, the blast radius
(every producer and consumer from the census), a confidence, and the principle it rests on.

Mark confidence "high" only when the evidence is concrete and no plainer shape is justified; otherwise
"low". No findings is a valid result: a well-modelled family should return none. Do not invent findings.`
}

function refuteLensPrompt(f) {
  return `Adversarially verify one model-level finding. Default to keeping it: a false positive is
cut cheaply by whoever reads the report, but a refuted true defect is never seen again. Refute only on
the evidence gate or a genuine misread.

  unit:         ${f.unit}
  lens:         ${f.lens}
  file:         ${f.file}:${f.line}
  evidence:     ${f.evidence}
  remodelling:  ${f.remodelling}
  blastRadius:  ${f.blastRadius}
  why:          ${f.why}

Read the actual type at ${f.file}:${f.line} and the sites the evidence cites.

REFUTE (refuted=true) only when one of these holds:
- EVIDENCE GATE: the finding lacks the concrete artefact its lens demands. L1 with no named illegal
  value; L2 with no named missing legal state and the workaround it forces; L3 with no disagreeing
  producer/consumer pair, both sites named, and the value one makes that the other cannot accept; L4
  with no specific boundary and type-role reason. "This feels off" is taste; refute it.
- MISREAD: the type does not have the claimed shape. The optional is genuinely optional in every role,
  the two roles are one role, the primitive has no domain constraint, or the boundary is internal or
  already parsed upstream. Or the finding belongs to another check: an L4 that is really a missing
  decodeUnknown call (effect-native-audit) or a *Sync decode inside an Effect (the language service).
  Say so and refute it here.

Never refute because:
- the remodelling's blast radius is large. Whether it is worth doing is decided after the audit; a real
  illegal state stays a finding however far it ripples. If the blast radius is wrong, correct it in
  your reason and keep the finding;
- the remodelling is imperfect but the evidence and direction are right. Keep it and note the
  correction;
- the remodelling names an API the pinned Effect lacks. Keep the finding and say so, so the
  remodelling is fixed rather than the defect lost.

When unsure, keep the finding (refuted=false).`
}

// ===========================================================================
// args may arrive parsed or as a JSON string.
const A = (() => {
  try {
    return typeof args === 'string' ? JSON.parse(args) : args || {}
  } catch {
    return {}
  }
})()
const date = A.date || 'undated'
const reportPath = A.reportPath || `target/effect-model-audit-${date}.md`
// All four lenses run by default; A.lenses, e.g. ['L3'], restricts the sweep.
const activeLenses = A.lenses ? LENSES.filter((l) => A.lenses.includes(l.id)) : LENSES

// ---------------------------------------------------------------------------
// DISCOVER. The enumeration and the grouping are separate agents, so the
// partition is checked against a list the grouping agent did not write.
// ---------------------------------------------------------------------------
phase('Discover')
log(`Effect-model audit ${date}: enumerating the domain types in ${SURFACE.join(', ')}`)

const enumeration = await agent(enumerationPrompt(), { label: 'discover:enumerate', phase: 'Discover', schema: ENUMERATION_SCHEMA, effort: 'low' })
const types = [...new Set(enumeration?.types ?? [])]
if (!types.length) {
  log('Enumeration returned no types; nothing to audit.')
  return { date, types, units: [], partitionOk: false, confirmed: [], lowConfidence: [], survivors: [], refutedCount: 0, coverage: [], failedCells: [] }
}

const grouping = await agent(groupingPrompt(types), { label: 'discover:group', phase: 'Discover', schema: GROUPING_SCHEMA })
let units = grouping?.units ?? []

const partition = assertPartition(types, units)
if (!partition.ok) {
  log(`Partition broken: missing=[${partition.missing.join(', ')}] duplicated=[${partition.duplicated.join(', ')}] unexpected=[${partition.unexpected.join(', ')}]. Repairing so coverage stays total.`)
  // Drop members that were never enumerated, keep each type in its first
  // family, and sweep anything still ungrouped into an `unassigned` family.
  const claimed = new Set()
  units = units.map((u) => ({
    ...u,
    members: (u.members || []).filter((m) => types.includes(m) && !claimed.has(m) && claimed.add(m)),
  })).filter((u) => u.members.length)
  const stillMissing = types.filter((t) => !claimed.has(t))
  if (stillMissing.length) {
    units = [...units, { key: 'unassigned', members: stillMissing, rationale: 'left ungrouped by the grouping pass', producers: [], consumers: [] }]
  }
  log(`After repair the partition is ok=${assertPartition(types, units).ok}: ${units.length} families cover ${types.length} types`)
} else {
  log(`Partition checked: ${units.length} families cover ${types.length} types exactly`)
}

// ---------------------------------------------------------------------------
// REASON then VERIFY, as a pipeline: each finding goes to its refuter as soon
// as its finder returns. A verifier that errors or returns nothing keeps the
// finding, flagged unverified.
// ---------------------------------------------------------------------------
phase('Reason')
const cells = units.flatMap((u) => activeLenses.map((l) => ({ u, l })))
log(`Reasoning over ${cells.length} cells (${units.length} families × ${activeLenses.length} lenses)`)

const verified = await pipeline(
  cells,
  ({ u, l }) => agent(lensFinderPrompt(u, l), { label: `reason:${u.key}:${l.id}`, phase: 'Reason', schema: LENS_FINDINGS_SCHEMA }).catch(() => null),
  (result, { u, l }) => {
    if (!result) return { unit: u.key, lens: l.id, failed: true, verified: [] }
    if (!result.findings.length) return { unit: u.key, lens: l.id, failed: false, verified: [] }
    return parallel(
      result.findings.map((f) => () =>
        agent(refuteLensPrompt(f), { label: `verify:${f.unit}:${f.lens}:${f.file}:${f.line}`, phase: 'Verify', schema: VERDICT_SCHEMA })
          .then((v) => ({ ...f, refuted: v ? v.refuted : false, verifyFailed: !v, refuteReason: v ? v.reason : 'the verifier returned no verdict, so the finding is kept unverified' }))
          .catch(() => ({ ...f, refuted: false, verifyFailed: true, refuteReason: 'the verifier errored, so the finding is kept unverified' })),
      ),
    ).then((vf) => ({ unit: u.key, lens: l.id, failed: false, verified: vf }))
  },
)

// A cell whose chain threw or came back empty is a failed cell, not a missing one.
const cellResults = verified.map((r, i) => r ?? { unit: cells[i].u.key, lens: cells[i].l.id, failed: true, verified: [] })
const allVerified = cellResults.flatMap((c) => c.verified).filter(Boolean)
const survivors = allVerified.filter((f) => !f.refuted)
const refutedCount = allVerified.filter((f) => f.refuted).length
const failedCells = cellResults.filter((c) => c.failed).map((c) => `${c.unit}:${c.lens}`)
const coverage = cellResults.map((c) => ({
  unit: c.unit,
  lens: c.lens,
  failed: !!c.failed,
  kept: c.verified.filter((f) => !f.refuted).length,
  refuted: c.verified.filter((f) => f.refuted).length,
}))

log(`Verified: ${survivors.length} kept, ${refutedCount} refuted across ${cellResults.length} cells${failedCells.length ? `; ${failedCells.length} cells failed (${failedCells.join(', ')}), so re-run them on resume` : ''}`)

// ---------------------------------------------------------------------------
// SYNTHESISE. The agent writes the report prose; the split comes from
// splitSurvivors.
// ---------------------------------------------------------------------------
phase('Synthesise')
let synthesis = null
let synthesisError = null
try {
  synthesis = await agent(
  `Synthesise the model-level type audit for ${date}. You are given the findings that survived
adversarial verification and the family × lens coverage.

Each finding carries: unit (type family), lens (L1 illegal-representable, L2 under-expressive, L3 role
coherence, L4 boundary honesty), file:line, the evidence, a proposed remodelling, and the blast radius.

1. MERGE: findings at one file:line may describe the same defect, since L1 and L3 overlap by design.
   Merge only findings that describe the same defect, keeping the clearest evidence and citing every
   lens that found it. Findings at one line that describe different defects stay separate rows, and
   findings at different lines that describe one defect may be merged.
2. SPLIT: a row with any "high" finding goes under Confirmed, anything else under Low-confidence.
3. Write the report to ${reportPath} with the Write tool, creating the directory if needed:
     ## Effect-model audit — ${date}
     ### Confirmed findings   <- table: file:line | unit | lens | evidence | remodelling | blastRadius | why
     ### Low-confidence
     ### Coverage             <- families × lenses run with each cell's kept and refuted counts, any failed cells, the number of types partitioned
4. Return reportMarkdown, identical to the file.

SURVIVORS (JSON):
${JSON.stringify(survivors, null, 2)}

COVERAGE (JSON):
${JSON.stringify({ typesCount: types.length, units: units.map((u) => u.key), cells: coverage, failedCells }, null, 2)}`,
  { label: 'synthesise', phase: 'Synthesise', schema: SYNTH_SCHEMA },
  )
} catch (e) {
  synthesisError = String((e && e.message) || e)
}

const { confirmed, lowConfidence } = splitSurvivors(survivors)

log(
  synthesis
    ? `Report written to ${reportPath}: ${confirmed.length} confirmed, ${lowConfidence.length} low-confidence`
    : `Synthesis failed, so no report was written; ${confirmed.length} confirmed and ${lowConfidence.length} low-confidence are returned for write-up`,
)

return {
  date,
  reportPath,
  reportWritten: !!synthesis,
  synthesisError,
  types,
  units: units.map((u) => ({ key: u.key, members: u.members })),
  partitionOk: assertPartition(types, units).ok,
  confirmed,
  lowConfidence,
  survivors,
  refutedCount,
  coverage,
  failedCells,
}
