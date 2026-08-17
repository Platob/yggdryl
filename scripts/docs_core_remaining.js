export const meta = {
  name: 'yggdryl-docs-core-remaining',
  description: 'Write the documentation from blank: one page per core module folder, every example in all three languages',
  phases: [
    { title: 'Core', detail: 'one agent per core module page' },
    { title: 'Guides', detail: 'landing, getting started, architecture, extensions, development' },
    { title: 'Verify', detail: 'run every example in all three languages and review every claim' },
    { title: 'Repair', detail: 'fix the pages verification rejected' },
  ],
}

// Agents run from the repository root, so the relative root works on any
// checkout. Pass a path as the workflow's args to point somewhere else.
const REPO = (typeof args === 'string' ? args : args?.repo) || '.'

const ARCHITECTURE = `
PROJECT yggdryl. Rust core at ${REPO}/rust (crate "yggdryl"), Python package at ${REPO}/python
(package "yggdryl"), Node package at ${REPO}/node ("@yggdryl/node"). The workspace manifest is at the
repository root; each member uses src/ tests/ benchmarks/ and there are NO examples/ directories -
runnable examples live only in the documentation.

TWO GENERATED INVENTORIES ARE YOUR GROUND TRUTH. Read them before writing a single name:
  ${REPO}/.api-inventory.txt  - every public Rust item with its signature, grouped by module
  ${REPO}/.api-bindings.txt   - every public Python and JavaScript name the two packages expose
If a name is not in the relevant inventory, IT DOES NOT EXIST. Do not write it.

ARCHITECTURE IN BRIEF:
- A NON-NULL STRUCT \`Field\` IS THE SCHEMA. There is no Record, RecordSchema, Tabular, ArrowTable,
  MediaDescriptor, IOMedia, RecordSettings, or BatchCastPlan, and no yggdryl::media, yggdryl::cast,
  or yggdryl::codec module. \`DataType\` is the logical type tree; \`Field\` adds name, nullability,
  and metadata.
- \`yggdryl::io::IOBase\` is the one storage trait: positional pread/pwrite, lazy by contract
  (constructing touches nothing, reading something absent yields nothing, writing creates).
  \`IOKind\` says what a handle addresses. The role traits \`IOPath\`/\`IOFile\`/\`IOFolder\`
  pre-implement what follows from a resource's role.
- \`yggdryl::generic\` holds one enum per contract: \`Holder\` (every IOBase), \`Media\` (every record
  encoding), \`Codec\` (every content coding over a handle), \`RecordOptions\` (every encoding's
  settings, with \`IORecordOptions\` as the shared settings trait and flat public fields).
- \`yggdryl::local\` supplies the three filesystem roles: \`Path\`, \`Folder\`, \`File\`.
- \`yggdryl::gzip|zlib|zstd\` each expose load/dump/reader/writer plus a transparent IOBase handle.
- \`yggdryl::ipc\` and \`yggdryl::parquet\` (non-default \`parquet\` feature) read and write ARROW
  BATCHES over any handle. Reading returns \`yggdryl::arrow::BatchReader\`, which STREAMS; writing
  takes any IntoIterator of batches. There is NO row-level read_records/write_records anywhere.
- \`yggdryl::arrow\` holds scalars (ArrowScalar, StructScalar, DefaultArrowScalar), schema projection
  (schema_from_field, record_schema_from_arrow, record_schema_to_arrow), and BatchReader.
- \`yggdryl::field::cast\` holds the ArrowCast trait and the typed per-datatype casts
  (Int64Field::cast_arrow_array -> Int64Array via ArrowFieldType). Batch casting IS array casting.
- \`yggdryl::text\` holds the shared \`Value\` tree and the four format types \`Json\`, \`Jsonl\`,
  \`Toml\`, \`Yaml\`, all implementing \`TextCodec\`.
- \`yggdryl::uri\` holds Uri/Url/Urn with full std Path/PathBuf interop.
- \`yggdryl::iceberg\` (non-default \`iceberg\` feature) maps Iceberg schemas to struct Fields.
`

const CONTRACT = `
DOCUMENTATION CONTRACT - follow every rule.

1. ONE LINE FIRST. Every page begins with a single H1, then ONE short sentence saying what the module
   is for. Not a paragraph. That sentence is what a reader sees in search results.

2. EXAMPLE FIRST. Immediately after that sentence comes the smallest runnable example. Explanation
   follows examples, never precedes them, and explains only what the example cannot show:
   invariants, tradeoffs, failure modes.

3. ALL THREE LANGUAGES. Every code example is a tab set with Rust, Python, and JavaScript, in that
   order, showing THE SAME operation:

       === "Rust"

           \`\`\`rust
           ...
           \`\`\`

       === "Python"

           \`\`\`python
           ...
           \`\`\`

       === "JavaScript"

           \`\`\`javascript
           ...
           \`\`\`

   Four-space indent inside the tab, blank line after each marker.

   CHECK ${REPO}/.api-bindings.txt before writing a Python or JavaScript tab. If the module is not
   exposed to those runtimes at all (storage, records, compression, iceberg are Rust-only today),
   then instead of faking tabs, put exactly this line directly under the one-sentence lead:

       !!! note "Rust only"
           The Python and JavaScript packages do not expose this module yet.

   and write the page with Rust examples alone. NEVER invent a binding API - a fabricated tab is
   worse than an absent one.

4. EVERY EXAMPLE RUNS. \`python scripts/check_docs_examples.py\` executes every rust, python, and
   javascript block in the docs:
   * Rust blocks become tests compiled with \`--features "parquet iceberg"\`. A block may be bare
     statements using \`?\` (it is wrapped in a function returning Result) or declare its own
     \`fn main() -> Result<(), Box<dyn std::error::Error>>\`.
   * Python blocks run under ${REPO}/python/.venv. Import from \`yggdryl\`.
   * JavaScript blocks run under node; \`require('@yggdryl/node')\` is rewired to this repository.
   * Every block must be SELF-CONTAINED: its own imports, no variable carried over from a previous
     block, and at least one assertion that proves what the prose claims.
   * A block that truly cannot stand alone is tagged \`rust,ignore\` / \`python,ignore\` /
     \`javascript,ignore\` and the prose says why.

5. TONE. Concrete, tight, no marketing, no hedging, no feature tours. Prefer several small examples
   over one long one. Use H2 sections.

6. LINKS. Relative links only, and only to pages in the list you are given.
`

const CORE_PAGES = [
  { module: 'datatype', title: 'DataType', bindings: true,
    read: 'rust/src/datatype/, rust/tests/datatype/, python/src/datatype.rs, python/tests/test_datatype.py, node/src/datatype.rs, node/tests/datatype.test.js',
    cover: 'Building and parsing logical types, nesting, decimals, temporal units, dictionaries, run-end encoding, unions and the variant alias, the id/kind vocabulary, Arrow projection, default values, and compatibility rewriting for spark/polars/pandas.' },
  { module: 'field', title: 'Field', bindings: true,
    read: 'rust/src/field/ (mod.rs, typed.rs, value.rs, diff.rs, parser.rs, serde.rs, arrow.rs, cast/), rust/tests/field/, python/src/field.rs, python/tests/test_field.py, node/src/field.rs, node/tests/field.test.js, node/tests/fields.test.js',
    cover: 'A field is a name, datatype, nullability, and metadata - and a non-null struct field is the schema. Cover construction, metadata as a mapping (including the reserved field:init and PARQUET:field_id keys and the http:* protocol properties), the typed field aliases, validating and canonicalizing a Value against a struct root, comparison through show_diff/show_diffs, and the casting surface (ArrowCast plus the typed per-datatype casts). This is the most important page on the site.' },
  { module: 'arrow', title: 'Arrow interoperability', bindings: true,
    read: 'rust/src/arrow/, rust/tests/default_scalar.rs, rust/tests/value_bounds.rs, rust/tests/batch_cast.rs, plus the arrow_scalar methods in python/src and node/src',
    cover: 'ArrowScalar and StructScalar, DefaultArrowScalar, schema_from_field, record_schema_from_arrow / record_schema_to_arrow, and the streaming BatchReader. Say plainly that the row-to-Arrow conversion layer was removed: Arrow speaks batches and scalars. Cover the materialization budgets that reject an oversized allocation before making it. Python and JavaScript expose arrow scalars and casting, so those get three tabs.' },
  { module: 'ipc', title: 'Arrow IPC', bindings: false,
    read: 'rust/src/ipc/ (mod.rs, tests.rs)',
    cover: 'Reading and writing Arrow IPC over any handle: the free functions (read_field, read_batches, write_batches), the stateful Ipc type with its options and cached schema, streamed reading through BatchReader, writing from any iterator, and the automatic content coding.' },
  { module: 'parquet', title: 'Apache Parquet', bindings: false,
    read: 'rust/src/parquet/ (mod.rs, metadata.rs, tests.rs)',
    cover: 'The non-default parquet feature: ParquetOptions (flat shared fields plus compression, max_row_group_size, key_value_metadata), reading and writing batches, footer statistics (row groups, split offsets, null counts, bounds), field-id round trips, and why a handle declaring an outer content coding is rejected.' },
  { module: 'iceberg', title: 'Apache Iceberg', bindings: false,
    read: 'rust/src/iceberg/ (mod.rs, types.rs, schema.rs, tests.rs)',
    cover: 'The non-default iceberg feature: PrimitiveType (the closed Iceberg vocabulary and its exact mapping to DataType, refusing what Iceberg cannot express), schema_from_json/schema_to_json (an Iceberg schema is a non-null struct Field whose children carry PARQUET:field_id, and requirement inverts into nullability), and assign_field_ids. State what is not here: no catalog client, no manifests, no transaction protocol.' },
  { module: 'uri', title: 'URI, URL, and URN', bindings: true,
    read: 'rust/src/uri.rs, rust/tests/uri.rs, python/src/uri.rs, python/tests/test_uri.py, node/src/uri.rs, node/tests/uri.test.js',
    cover: 'Parsing and canonical syntax, components, path segments and extensions, media-type inference from a compound filename, joinpath/parent/parents/parts, default_port, and the std Path/PathBuf interop (from_path, to_path, TryFrom, join_path, is_local, exists, is_dir, is_file, local_mime_type). All three languages expose this.' },
  { module: 'text', title: 'Structured text values', bindings: true,
    read: 'rust/src/text/ (mod.rs, value.rs, codec.rs, format.rs, limits.rs, display.rs, io.rs), rust/tests/text/, python/src/codec.rs, node/src/codec.rs',
    cover: 'The shared Value tree - variants, floats, limits, byte positions, format inference - and then the four format types Json/Jsonl/Toml/Yaml behind one TextCodec surface, including reading and writing through an IOBase handle with its content coding. Python and JavaScript expose the value conversion and the codecs, so those examples get three tabs.' },
  { module: 'json', title: 'JSON', bindings: true,
    read: 'rust/src/json/, rust/tests/json.rs, python/yggdryl/json/, node/tests/codec.test.js',
    cover: 'Reading and writing JSON through the shared Value: whole-value and streaming forms, newline-delimited JSON, limits, and automatic content coding from a filename.' },
  { module: 'yaml', title: 'YAML', bindings: true,
    read: 'rust/src/yaml/, rust/tests/yaml.rs, python/yggdryl/yaml/, node/tests/codec.test.js',
    cover: 'Reading and writing YAML through the shared Value, multi-document handling, the deliberate absence of tag emission, and how a tag on input is read.' },
  { module: 'toml', title: 'TOML', bindings: true,
    read: 'rust/src/toml/, rust/tests/toml.rs, python/yggdryl/toml/, node/tests/toml.test.js',
    cover: 'Reading and writing TOML through the shared Value, table ordering, and the type mapping.' },
]

const CORE_LINKS = CORE_PAGES.map((page) => `${page.module}.md`).join(', ')

phase('Core')
log(`Writing ${CORE_PAGES.length} core pages, one per module folder`)

const PAGE_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['path', 'lead', 'headings', 'languages', 'blocks', 'uncertain'],
  properties: {
    path: { type: 'string' },
    lead: { type: 'string', description: 'the one-sentence lead you wrote' },
    headings: { type: 'array', items: { type: 'string' } },
    languages: { type: 'array', items: { type: 'string' } },
    blocks: { type: 'integer', description: 'total fenced example blocks' },
    uncertain: { type: 'array', items: { type: 'string' } },
  },
}

const core = await parallel(
  CORE_PAGES.map((page) => () =>
    agent(
      `${ARCHITECTURE}

${CONTRACT}

You own EXACTLY ONE file, which does not exist yet: ${REPO}/docs/core/${page.module}.md.
It documents the Rust module \`yggdryl::${page.module}\` (source: rust/src/${page.module}).

Title: "${page.title}"
${page.bindings
  ? `This module IS exposed to Python and JavaScript - every example needs all three tabs, and you must check ${REPO}/.api-bindings.txt for the exact names.`
  : 'This module is Rust-only: use the "Rust only" note from the contract and write Rust examples alone.'}

Read first: ${REPO}/.api-inventory.txt (the yggdryl::${page.module} sections)${page.bindings ? ` and ${REPO}/.api-bindings.txt` : ''}.
Then read: ${page.read}.
What the page must cover: ${page.cover}

Other core pages you may link to (all will exist, relative from docs/core/): ${CORE_LINKS}.
You may also link to ../index.md, ../getting-started.md, ../architecture.md,
../extensions/python.md, ../extensions/javascript.md.

Method:
1. Read the inventories, then the sources and their tests. Test bodies and doctests are where the
   examples that actually compile live - adapt those.
2. Write the page: one H1, one sentence, then the smallest example with all its tabs.
3. Re-read with the inventories open and delete anything you cannot point at.
4. Check every block against the runnable rules: self-contained, own imports, an assertion.

Report your lead sentence, the headings, which languages appear, how many blocks you wrote, and
anything you stated without being able to verify it.`,
      { label: `core:${page.module}`, phase: 'Core', schema: PAGE_SCHEMA },
    ),
  ),
)

log(`Core: ${core.filter(Boolean).length}/${CORE_PAGES.length} pages written`)

const guides = []

phase('Verify')

const RUN_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['findings', 'summary'],
  properties: {
    summary: { type: 'string' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['page', 'language', 'problem', 'fix'],
        properties: {
          page: { type: 'string' },
          language: { type: 'string' },
          problem: { type: 'string' },
          fix: { type: 'string' },
        },
      },
    },
  },
}

const REVIEW_SCHEMA = {
  type: 'object',
  additionalProperties: false,
  required: ['findings', 'pagesChecked'],
  properties: {
    pagesChecked: { type: 'array', items: { type: 'string' } },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        additionalProperties: false,
        required: ['page', 'claim', 'why', 'severity'],
        properties: {
          page: { type: 'string' },
          claim: { type: 'string' },
          why: { type: 'string' },
          severity: { type: 'string', enum: ['invented-api', 'wrong-behavior', 'missing-tabs', 'no-lead-sentence', 'stale-architecture', 'broken-link', 'style'] },
        },
      },
    },
  },
}

const REVIEW_GROUPS = [
  { name: 'schema', pages: ['docs/core/enums.md', 'docs/core/datatype.md', 'docs/core/field.md', 'docs/core/arrow.md'] },
  { name: 'storage', pages: ['docs/core/io.md', 'docs/core/generic.md', 'docs/core/local.md', 'docs/core/gzip.md', 'docs/core/zlib.md', 'docs/core/zstd.md'] },
  { name: 'records', pages: ['docs/core/ipc.md', 'docs/core/parquet.md', 'docs/core/iceberg.md'] },
  { name: 'values', pages: ['docs/core/uri.md', 'docs/core/text.md', 'docs/core/json.md', 'docs/core/yaml.md', 'docs/core/toml.md'] },
  { name: 'guides', pages: ['docs/index.md', 'docs/getting-started.md', 'docs/architecture.md', 'docs/extensions/python.md', 'docs/extensions/javascript.md', 'docs/testing.md', 'docs/benchmarks.md', 'docs/contributing.md'] },
]

const verification = await parallel([
  () =>
    agent(
      `${ARCHITECTURE}

Run the documentation example checker and report every failure:

  cd ${REPO} && python scripts/check_docs_examples.py

It compiles and runs every rust, python, and javascript block under docs/. Do NOT edit any page -
repairs happen in a later phase. For each failure, name the page, the language, the block index, the
precise problem, and the exact fix (the real name from ${REPO}/.api-inventory.txt or
${REPO}/.api-bindings.txt, the missing import, or the wrong assertion).

Report every failing block plus a one-paragraph summary of the run.`,
      { label: 'verify:run', phase: 'Verify', schema: RUN_SCHEMA },
    ),
  ...REVIEW_GROUPS.map((group) => () =>
    agent(
      `${ARCHITECTURE}

You are REVIEWING freshly written documentation. Refute it; do not praise it.

Pages: ${group.pages.join(', ')} (relative to ${REPO}).

Check every claim against ${REPO}/.api-inventory.txt, ${REPO}/.api-bindings.txt, and the sources.
Report, in priority order:

- INVENTED API: a name, argument, or module path not in an inventory. Quote the exact text.
- WRONG BEHAVIOR: a stated result or invariant the code does not produce.
- MISSING TABS: an example that is not shown in all three languages, on a page whose module IS
  exposed to Python and JavaScript. (A page carrying the "Rust only" note is exempt, but check that
  the note is justified against .api-bindings.txt.)
- NO LEAD SENTENCE: the page does not open with H1 followed by exactly one short sentence.
- STALE ARCHITECTURE: Record, RecordSchema, Tabular, ArrowTable, MediaDescriptor, IOMedia,
  RecordSettings, BatchCastPlan, read_records/write_records, yggdryl::media, yggdryl::cast,
  yggdryl::codec, the name "rekep", the mmap feature, or benches/examples directories.
- BROKEN LINK: a relative link with no target file under ${REPO}/docs.
- STYLE: explanation before the example, hedging, or a feature tour.

Do NOT edit the pages. Report the offending text and the correct replacement.`,
      { label: `review:${group.name}`, phase: 'Verify', schema: REVIEW_SCHEMA },
    ),
  ),
])

const runReport = verification[0]
const runFindings = (runReport && runReport.findings) || []
const reviewFindings = verification.slice(1).filter(Boolean).flatMap((review) => review.findings || [])

log(`Verify: ${runFindings.length} failing examples, ${reviewFindings.length} review findings`)

phase('Repair')

const work = new Map()
for (const finding of runFindings) {
  if (!work.has(finding.page)) work.set(finding.page, { run: [], review: [] })
  work.get(finding.page).run.push(finding)
}
for (const finding of reviewFindings) {
  if (!work.has(finding.page)) work.set(finding.page, { run: [], review: [] })
  work.get(finding.page).review.push(finding)
}

const repairs = await parallel(
  [...work.entries()].map(([page, items]) => () =>
    agent(
      `${ARCHITECTURE}

${CONTRACT}

You own EXACTLY ONE file: ${REPO}/${page}. Fix the problems below and change nothing else. Do not
weaken the page to make a problem go away - a page that documents less is not a fixed page.

Failing examples:
${JSON.stringify(items.run, null, 2)}

Review findings:
${JSON.stringify(items.review, null, 2)}

Verify each report yourself against ${REPO}/.api-inventory.txt, ${REPO}/.api-bindings.txt, and the
source before acting; some reports will be wrong, and you should say so rather than break a correct
page.

Then verify your own work:
  cd ${REPO} && python scripts/check_docs_examples.py 2>&1 | tail -60
Every block of YOUR page must pass. Other pages may still fail - those belong to other agents.`,
      { label: `repair:${page.split('/').pop()}`, phase: 'Repair', schema: {
        type: 'object',
        additionalProperties: false,
        required: ['page', 'fixed', 'rejected', 'notes'],
        properties: {
          page: { type: 'string' },
          fixed: { type: 'integer' },
          rejected: { type: 'array', items: { type: 'string' } },
          notes: { type: 'string' },
        },
      } },
    ),
  ),
)

log(`Repair: ${repairs.filter(Boolean).length} pages revisited`)

return {
  core: core.filter(Boolean).map((page) => ({ path: page.path, lead: page.lead, languages: page.languages, blocks: page.blocks })),
  guides: guides.filter(Boolean).map((page) => ({ path: page.path, lead: page.lead, blocks: page.blocks })),
  runFindings,
  reviewFindings,
  repairs: repairs.filter(Boolean),
}
