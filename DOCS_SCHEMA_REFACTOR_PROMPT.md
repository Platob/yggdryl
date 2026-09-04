# Schema documentation refactor brief

Split the Schema documentation into one `DataType` tree and one `Field` tree, each
generic-first and then one page per family. Follow `AGENTS.md`; this file contains only
refactor-specific decisions. No Rust, Python, or JavaScript source changes.

## Outcome

- `docs/schema/datatype/` and `docs/schema/field/`, each an index page holding the shared
  contract plus family pages holding only what differs.
- Every page reads whole in one sitting: purpose sentence, at-a-glance table, then sections
  shaped contract line -> example -> measured number.
- Examples and measured tables outnumber prose. Every fence still runs under
  `scripts/check_docs_examples.py`.
- Benchmark tables sit on the page whose method they measure; `docs/benchmarks.md` points at
  the new anchors.
- `mkdocs.yml` nav, every inbound link, the `AGENTS.md` documentation rules, and
  `scripts/docs_core_remaining.js` groups move in the same change. `docs/datatype.md` and
  `docs/field.md` are deleted, never stubbed or redirected.

## Read first

- `AGENTS.md`: "Compress everything", "Optimize documentation for lookup", "Documentation
  layout", "Verification".
- `.api-inventory.txt` and `.api-bindings.txt`. A name absent from the relevant inventory does
  not exist; never write it.
- `docs/datatype.md`, `docs/field.md` - the 2 889 lines being redistributed.
- `rust/src/datatype/` and `rust/src/field/` - the family modules that already define the split.
- `rust/src/generic/datatype_kind.rs` - `DataTypeKind::ALL` is the category vocabulary. Do not
  invent a category outside it.
- `rust/benchmarks/{datatype,field}/`, `python/benchmarks/`, `node/benchmarks/` - the targets
  behind every published number.
- `scripts/check_docs_examples.py` - fence and `ignore` rules.

## Target tree

`DataType`, under `docs/schema/datatype/`:

| page | covers | budget |
| --- | --- | ---: |
| `index.md` | the contract every type shares: canonical text and aliases, JSON, `DataTypeId`/`DataTypeKind`, child access, defaults, Arrow projection, compatibility rewriting, rendering, Rust-only `validate`. Ends with the family table. `Null` and `Boolean` are documented in that table - they take no parameters and get no page. | 220 lines |
| `integer.md` | signed and unsigned widths, ranges, dictionary-key duty | 160 |
| `floating.md` | `Float16`/`32`/`64`, resolution picks the width | 140 |
| `decimal.md` | `decimal128`/`decimal256`, precision and scale rules | 160 |
| `temporal.md` | date, time, timestamp, duration, interval; `TimeUnit`, `TimeZone` | 200 |
| `string.md` | `Utf8`, large and view layouts, the three ASCII widths, `AsciiDictionary` and its generated enum | 200 |
| `binary.md` | binary, large, view, fixed-size layouts | 140 |
| `nested.md` | list family, struct, map, union and the dense-union sugar; per-family child counts | 220 |
| `encoded.md` | dictionary and run-end encodings that wrap a value | 160 |
| `variant.md` | self-describing values and their extension identity | 140 |
| `geospatial.md` | geometry and geography, WKB payloads, edge algorithms | 160 |

`Field`, under `docs/schema/field/`:

| page | covers | budget |
| --- | --- | ---: |
| `index.md` | name, type, nullability, metadata in one value; item access reaching a child and never metadata; serialization; rendering. Ends with the page table. | 200 lines |
| `schema.md` | the non-null struct root: flatten, expand, merge, compare and diff | 220 |
| `metadata.md` | the metadata mapping, reserved keys, protocol properties, one protocol at a time, partition columns | 220 |
| `typed.md` | typed field aliases, the binding `fields` factories, converting to one native field, cached-access cost | 200 |
| `values.md` | row validation and canonicalization against the root (Rust-only) | 140 |
| `cast.md` | casting Arrow data through a field, then the generic cast | 220 |

The new tree must total fewer lines than the two pages it replaces. If it does not, the split is
repeating itself.

`docs/arrow.md` and `docs/playground.md` stay where they are and stay under the Schema nav
section. They are not part of this split, and moving them costs inbound links for no reader gain.

## Generic first, specific after

The index page owns behavior that is true of every member. A family page owns only what a reader
cannot predict from the index:

- parameters and what picking one costs;
- family-only constructors, typed markers, and canonical spellings;
- family-only Arrow or extension identity;
- the family's default value and any compatibility exception;
- the family's benchmark.

A family page never restates a generic rule. It links to the index anchor. One fact lives in one
place. Open a family page with one line pointing back, for example: `Parsing, defaults, and Arrow
projection are the same for every type - see the shared contract.`

## Page shape

1. One H1 and one purpose sentence. No lead paragraph.
2. An at-a-glance table, at most 12 rows: canonical form, accepted spellings, Arrow type,
   default. This is what a reader scans before deciding to read.
3. At most 6 H2 sections. Each one: a single contract sentence, then the example, then at most
   two sentences of non-obvious edge case.
4. At most 3 sentences of prose per section, at most 2 before the first example.
5. Every section carries at least one runnable example. A section with no example is a table row
   on another page.
6. Measured tables last on the page that owns the method.

Cut anything that narrates a signature, repeats an example in prose, or explains the design twice.
Keep the existing voice: short declarative sentences, canonical symbol names, stable headings.

## Examples

- Tabs in Rust, Python, JavaScript order, the same operation expressed idiomatically in each.
- Mark a Rust-only surface explicitly. Never invent a binding to fill a tab.
- Every block is self-contained, carries an assertion, and passes
  `python scripts/check_docs_examples.py`. Ignored blocks use the brace superfence form.
- Prefer the smallest example that proves the contract. Prefer a second example over a paragraph.
- Reuse examples that already pass. Rewrite only to shorten or to fix a broken import after a move.

## Benchmarks

- Move each measured table verbatim, with its command block and its host sentence. Never re-round,
  re-word, or re-attribute a number.
- A new table requires a real release run on the host doing the work, named the way the existing
  tables name machine, runtime, and build. If a run cannot be made here, leave the section out and
  report it as skipped. Never write a number you did not measure.
- Point each family page at its own target and filter, for example
  `cargo bench -p yggdryl --bench datatype --all-features -- temporal`. Map
  `rust/benchmarks/datatype/{temporal,floating,geospatial,ascii,nested,parser,default,arrow,value}.rs`
  and `rust/benchmarks/field/{integer,comparison,parser,arrow,value}.rs` to the pages that own them.
- No benchmark-only page. A table lives with the method it prices.
- Update `docs/benchmarks.md`: both the link list and the "What each target isolates" table.

## Content migration

Every existing section lands in exactly one place. Nothing is dropped silently; report anything
deliberately deleted as redundant.

| `docs/datatype.md` | destination |
| --- | --- |
| lead, `Children` | `datatype/index.md`, per-family child counts to `nested.md` |
| `Precision and resolution pick the width` | `decimal.md` and `temporal.md` |
| `Encodings that wrap a value` | `encoded.md` |
| `Unions and the dense-union sugar` | `nested.md` |
| `Variant, geometry, and geography` | `variant.md` and `geospatial.md` |
| `ASCII widths`, `The dictionary vocabulary and its generated enum` | `string.md`, cross-linked from `encoded.md` |
| `Logical names`, `Identity and family`, `Arrow projection` | `datatype/index.md` |
| `Default values` | rule to `datatype/index.md`, family rows to family pages |
| `Serializing a schema`, `A readable rendering`, `Compatibility rewriting`, `Building the enum directly` | `datatype/index.md` |

| `docs/field.md` | destination |
| --- | --- |
| lead, `Item access reaches a child, never metadata`, `Serializing a schema`, `A readable rendering` | `field/index.md` |
| `A non-null struct field is the schema`, `Flattening and expanding`, `Merging two schemas`, `Comparing two fields` | `field/schema.md` |
| `Metadata is a mapping`, `Reserved keys and protocol properties`, `One protocol at a time`, `A field can be a partition column` | `field/metadata.md` |
| `Typed field aliases`, `Converting to one native field`, `What cached field access costs` | `field/typed.md` |
| `Row values are validated against the root` | `field/values.md` |
| `Casting Arrow data through a field`, `The generic cast` | `field/cast.md` |

## Navigation and links

- `mkdocs.yml` nav keeps the lowercase leaf titles the file already uses, and relies on the
  enabled `navigation.indexes` feature for the two index pages:

  ```yaml
  - Schema:
      - Data types:
          - schema/datatype/index.md
          - integers: schema/datatype/integer.md
          # one entry per family page, in DataTypeKind::ALL order
      - Fields:
          - schema/field/index.md
          # one entry per field page
      - arrow: arrow.md
      - playground: playground.md
  ```

- 52 references under `docs/` point at `datatype.md` or `field.md`: 41 from 17 other pages, the
  rest internal to the two pages being split. Repoint every one at the page that now owns the
  fact, anchor included, never at an index page when a family page owns it.
- Outside `docs/`: `README.md:28`, `rust/README.md:65`, `python/FIELDS.md:54,144`, and the
  `schema` group in `scripts/docs_core_remaining.js:266`. That group also still names the retired
  `docs/enums.md`; list the real pages.
- Leave the root `*_PROMPT.md` briefs alone. They are history, not documentation.
- No stub page, no redirect, no "moved here" note. `git mv` where a page has one destination.

## AGENTS.md

Amend the "Documentation layout" section in the same change, minimally:

- "One page per core module" stays for every other domain. Add that schema is documented as two
  trees under `docs/schema/`, each an index page owning the shared contract and one page per
  `DataTypeKind` family or `Field` concern, and that a family page never restates a generic rule.
- State the page shape and the prose budget once, where the other layout rules live.

## Non-goals

- No Rust, Python, or JavaScript source change. No new public API.
- No new claim about behavior. If a sentence cannot be traced to the inventories or to source,
  delete it rather than carry it over.
- No moving `arrow.md`, `playground.md`, or any non-schema page.
- No marketing text, no reader-facing migration note, no benchmark-only page.

## Verification

Run and report exact results, including anything skipped:

```console
python scripts/check_docs_examples.py
python -m mkdocs build --strict
grep -rn "datatype\.md\|field\.md" docs README.md rust/README.md python/FIELDS.md scripts/
```

The grep must return only intended `schema/...` paths. `--strict` fails on a broken link or a
dead anchor, so treat its output as the link check.

## Done when

- [ ] `docs/schema/datatype/` and `docs/schema/field/` exist with the pages above; the old two
      pages are deleted.
- [ ] Every page is inside its budget, has one at-a-glance table, and every section has an example.
- [ ] No family page restates a generic rule; each links back instead.
- [ ] Every example passes the checker in all three languages, or is a marked Rust-only surface.
- [ ] Every measured table is verbatim or freshly measured, with its command and host sentence.
- [ ] `mkdocs.yml`, `docs/benchmarks.md`, `AGENTS.md`, `scripts/docs_core_remaining.js`, and all
      inbound links updated; `mkdocs build --strict` is clean.
- [ ] The new tree is smaller than the two pages it replaces.
