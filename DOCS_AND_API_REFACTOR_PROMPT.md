# Documentation and API refactor brief

Three changes that ship together: a type stops being spelled in snake_case, per-language default
projections collapse into one `Scalar`, and the Schema and Generic documentation become
generic-first trees. Follow `AGENTS.md`; this file holds only refactor-specific decisions.

## Delivery order

Rust core, then Python and JavaScript, then documentation. Parts 1 and 2 change names the
documentation quotes, so land them before rewriting a page: a page showing a name Part 1 removed
is a broken page, and `scripts/check_docs_examples.py` will say so.

No deprecated alias, no accepted old spelling, no dual behavior, no migration note. Update every
caller directly, in the same change.

## Read first

- `AGENTS.md`: "Compress everything", "Optimize documentation for lookup", "Documentation
  layout", "Binding boundary", "Verification".
- `.api-inventory.txt` and `.api-bindings.txt`. A name absent from the relevant inventory does
  not exist; never write it. Both move with the surface they describe.
- `rust/src/generic/enum_scalar.rs`, `rust/src/generic/mod.rs` - the enum vocabulary and its
  identity strings.
- `rust/src/datatype/default.rs`, `rust/src/field/mod.rs` - the one default the core computes.
- `python/src/{datatype,field,scalar}.rs`, `python/yggdryl/fields/_defaults.py`,
  `node/src/{datatype,field}.rs`, `node/defaults.js`, `node/binding.d.ts` - the per-language
  projections Part 2 removes.
- `docs/datatype.md`, `docs/field.md`, `docs/generic.md` - the 3 691 lines Part 3 redistributes.
- `rust/src/{datatype,field}/` and `rust/src/generic/` - the module split the pages follow.
- `rust/src/generic/datatype_kind.rs` - `DataTypeKind::ALL` is the category vocabulary. Do not
  invent a category outside it.
- `rust/benchmarks/{datatype,field}/`, `python/benchmarks/`, `node/benchmarks/` - the targets
  behind every published number.

---

# Part 1 - a type is spelled like the type

## Rule

A string that names a type spells the type exactly: `IOMode`, never `io_mode`; `DataTypeId`,
never `data_type_id`. This holds in every language, in serialized bytes, and in error text.

Names that identify a member, method, field, or collection keep their host language's
convention. `enum_kind` stays `enum_kind` in Python and `enumKind` in JavaScript, the expression
selector `&holder.mime_type` stays an attribute, and the member collections `enums.DATA_TYPE_IDS`
and `enums.dataTypeIds` stay as they are - they name a set of values, not a type.

## Known offenders

| location | now | becomes |
| --- | --- | --- |
| `EnumScalar::from_parts` and `EnumScalar::kind` | `codec`, `data_type_id`, `data_type_kind`, `edge_algorithm`, `io_kind`, `io_mode`, `time_unit`, `union_mode` | `Codec`, `DataTypeId`, `DataTypeKind`, `EdgeAlgorithm`, `IOKind`, `IOMode`, `TimeUnit`, `UnionMode` |
| `#[serde(tag = "kind", content = "value", rename_all = "snake_case")]` on `EnumScalar` | snake_case tags on the wire | drop `rename_all`; the variant names already are the type names |
| `Scalar.from_enum` / `Scalar.fromEnum` arguments, `enum_kind` / `enumKind` results | `"io_mode"` | `"IOMode"` |
| every doc example, test, and benchmark quoting one of those strings | | the exact spelling |

- The kind match becomes exact. Member spellings keep the case-insensitive match they have today
  (`value.eq_ignore_ascii_case(member.as_str())`); only the vocabulary name is exact.
- A document written with the old tags stops parsing. That is the intended break, not a
  regression to soften with a fallback.
- Sweep both inventories for any other public name that mangles a type: a class, wrapper, module,
  metadata key, error target, or serialized tag. Fix what stands for a type; leave the rest.
- `AGENTS.md` names `TimeZone`; the crate exports `Timezone`. One spelling wins and the loser is
  updated in the same change.

---

# Part 2 - one default, one Scalar

## Rule

The core computes one default and returns one `Scalar`. Conversion to a host value belongs to
`Scalar` alone. No `DataType`, `Field`, or any other type carries a per-language projection.

## Surface

| language | remove | keep |
| --- | --- | --- |
| Rust | `DataType::default_value`, `Field::default_value`, `is_default_value` (renamed, not aliased) | `default_scalar() -> Result<Scalar>`, `is_default_scalar(&Scalar) -> Result<bool>` |
| Python | `DataType.default_pyvalue`, `Field.default_pyvalue`, both `default_pyhint` | `default_scalar() -> Scalar`, then the caller's own `scalar.as_py()` |
| JavaScript | `defaultJSValue`, `defaultJSHint` on both classes, plus `_defaultJSValueNative` and `_defaultJSHintNative` | `defaultScalar(): Scalar`, then the caller's own `scalar.asJs()` |

- One name across the three languages: `default_scalar`, `default_scalar`, `defaultScalar`. It
  says what it returns, and it already matches the `rust/tests/default_scalar.rs` target name.
- The Arrow boundary stays: `default_arrow_scalar`, `defaultArrowScalar`, `default_arrow_array`.
  Arrow is a shared exchange boundary, not a language projection.
- Delete the machinery that existed only to feed the removed methods: `python/yggdryl/fields/_defaults.py`
  and its hint plumbing, `node/defaults.js`, `dtype_js_hint` / `JsValueHint`, and the
  `node/binding.d.ts` entries including the exported `JSValueHint` interface. Whatever the `@scalar` dataclass boundary still needs stays there
  and is reachable from that boundary only.
- Before deleting, check `Scalar.as_py` and `Scalar.asJs` against every family: struct, list,
  fixed-size list, map, union, decimal, temporal, ASCII, geospatial, variant. A gap is fixed once
  inside `Scalar`, never by keeping a schema-side method.
- The typed dataclass materialization `default_pyvalue` performed is not reproduced. `as_py`
  returns the plain Python value; a caller wanting an instance builds it from its own class. If
  that projection is worth keeping, it belongs on the `@scalar` class, not on `Field` - raise it
  rather than reintroducing it here.
- Update every caller: `python/tests/{test_defaults,test_datatype,typing_bindings,typing_fields}.py`,
  the Node tests, `node/benchmarks/defaults.js`, the Python benchmarks, and both inventories.

---

# Part 3 - documentation trees

Reorganize `docs/datatype.md`, `docs/field.md`, and `docs/generic.md` into three generic-first
trees: an index page owning what every member shares, then one short page per family or concern.

## Schema tree

`DataType`, under `docs/schema/datatype/`:

| page | covers | budget |
| --- | --- | ---: |
| `index.md` | the contract every type shares: canonical text and aliases, JSON, `DataTypeId`/`DataTypeKind`, child access, defaults, Arrow projection, compatibility rewriting, rendering, Rust-only `validate`. Ends with the family table. `Null` and `Boolean` are documented in that table - they take no parameters and get no page. | 220 lines |
| `integer.md` | signed and unsigned widths, ranges, dictionary-key duty | 160 |
| `floating.md` | `Float16`/`32`/`64`, resolution picks the width | 140 |
| `decimal.md` | `decimal128`/`decimal256`, precision and scale rules | 160 |
| `temporal.md` | date, time, timestamp, duration, interval; `TimeUnit`, `Timezone` | 200 |
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

`docs/arrow.md` and `docs/playground.md` stay where they are and stay under the Schema nav
section. They are not part of this split, and moving them costs inbound links for no reader gain.

## Generic tree

Under `docs/generic/`:

| page | covers | budget |
| --- | --- | ---: |
| `index.md` | what lives in `generic` and why one enum sits beside each trait: the shared vocabulary table, `EnumScalar` identity with its measured boundary, `MAGIC_PROBE_LEN` and inference ownership. Ends with the page table. | 220 lines |
| `scalar.md` | the value every part of the project speaks: families, the shared `as_integer`/`as_float`/`as_decimal`/`as_temporal` views, and the one host conversion (`as_py`/`from_py`, `asJs`/`fromJs`) Part 2 makes the only path | 220 |
| `typed.md` | `TypedScalar` and its per-datatype aliases (Rust-only) | 140 |
| `holder.md` | `Holder`: every storage handle behind one value | 160 |
| `codec.md` | `Codec`: a coding over a handle | 140 |
| `media.md` | `Media`: a record encoding over a handle, with the measured redirection | 180 |
| `options.md` | `RecordOptions`, `IORecordOptions`, `RecordSettings` | 220 |
| `wkb.md` | the WKB reader (Rust-only) | 120 |

`Scalar` parsing and rendering stay in [`docs/text.md`](docs/text.md); `generic/scalar.md` owns
the value model and links there. Do not duplicate text.md, and do not empty it.

Each tree must total fewer lines than the page it replaces. If it does not, the split is
repeating itself.

## Generic first, specific after

An index page owns behavior true of every member. A leaf page owns only what a reader cannot
predict from the index:

- parameters, and what picking one costs;
- family-only constructors, typed markers, and canonical spellings;
- family-only Arrow or extension identity;
- the family's default value and any compatibility exception;
- the family's benchmark.

A leaf page never restates a generic rule; it links to the index anchor. One fact lives in one
place. Open a leaf page with one line pointing back, for example: `Parsing, defaults, and Arrow
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
6. Measured tables last, on the page that owns the method.

Cut anything that narrates a signature, repeats an example in prose, or explains the design
twice. Keep the existing voice: short declarative sentences, canonical symbol names, stable
headings.

## Examples

- Tabs in Rust, Python, JavaScript order, the same operation expressed idiomatically in each.
- Mark a Rust-only surface explicitly. Never invent a binding to fill a tab.
- Every block is self-contained, carries an assertion, and passes
  `python scripts/check_docs_examples.py`. Ignored blocks use the brace superfence form.
- Prefer the smallest example that proves the contract. Prefer a second example over a paragraph.
- Reuse examples that already pass. Rewrite only to shorten, to fix an import after a move, or to
  carry a Part 1 or Part 2 rename.

## Benchmarks

- Move each measured table verbatim, with its command block and its host sentence. Never
  re-round, re-word, or re-attribute a number.
- A new table requires a real release run on the host doing the work, named the way the existing
  tables name machine, runtime, and build. If a run cannot be made here, leave the section out
  and report it as skipped. Never write a number you did not measure.
- Point each leaf page at its own target and filter, for example
  `cargo bench -p yggdryl --bench datatype --all-features -- temporal`. Map
  `rust/benchmarks/datatype/{temporal,floating,geospatial,ascii,nested,parser,default,arrow,value}.rs`
  and `rust/benchmarks/field/{integer,comparison,parser,arrow,value}.rs` to the pages that own
  them.
- No benchmark-only page. A table lives with the method it prices.
- Update `docs/benchmarks.md`: the link list and the "What each target isolates" table.

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
| `Default values` | rule to `datatype/index.md`, family rows to family pages; rewritten for Part 2 |
| `Serializing a schema`, `A readable rendering`, `Compatibility rewriting`, `Building the enum directly` | `datatype/index.md` |

| `docs/field.md` | destination |
| --- | --- |
| lead, `Item access reaches a child, never metadata`, `Serializing a schema`, `A readable rendering` | `field/index.md` |
| `A non-null struct field is the schema`, `Flattening and expanding`, `Merging two schemas`, `Comparing two fields` | `field/schema.md` |
| `Metadata is a mapping`, `Reserved keys and protocol properties`, `One protocol at a time`, `A field can be a partition column` | `field/metadata.md` |
| `Typed field aliases`, `Converting to one native field`, `What cached field access costs` | `field/typed.md` |
| `Row values are validated against the root` | `field/values.md` |
| `Casting Arrow data through a field`, `The generic cast` | `field/cast.md` |

| `docs/generic.md` | destination |
| --- | --- |
| lead, `Shared vocabulary` with its `EnumScalar` examples and measured table | `generic/index.md`, rewritten for Part 1 |
| `Holder: every storage handle` | `generic/holder.md` |
| `Codec: a coding over a handle` | `generic/codec.md` |
| `Media: a record encoding over a handle`, `Measured generic media redirection` | `generic/media.md` |
| `RecordOptions: every encoding's settings` | `generic/options.md` |
| `Scalar families` | `generic/scalar.md`, extended with the Part 2 conversion path |
| `TypedScalar: one value and its datatype` | `generic/typed.md` |
| `The WKB reader` | `generic/wkb.md` |

## Navigation and links

- `mkdocs.yml` nav keeps the lowercase leaf titles the file already uses, and relies on the
  enabled `navigation.indexes` feature for the index pages:

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
  - Generic:
      - generic/index.md
      - scalar: generic/scalar.md
      # one entry per generic page
  ```

- Inbound links to repoint, anchor included, at the page that now owns the fact - never at an
  index page when a leaf page owns it: 52 references to `datatype.md` or `field.md` under `docs/`
  (41 of them from 17 other pages) and 47 to `generic.md` (19 pages).
- Outside `docs/`: `README.md:28-29`, `rust/README.md:65`, `python/FIELDS.md:54,144`, and the
  `schema` and `storage` groups in `scripts/docs_core_remaining.js:266-267`. The `schema` group
  also still names the retired `docs/enums.md`; list the real pages.
- Leave the root `*_PROMPT.md` briefs alone. They are history, not documentation.
- No stub page, no redirect, no "moved here" note. `git mv` where a page has one destination.

---

## AGENTS.md

Amend it in the same change, minimally, so the authority matches the result:

- "Documentation layout": "One page per core module" stays for every other domain. Add that
  schema and generic are documented as trees under `docs/schema/` and `docs/generic/`, each an
  index page owning the shared contract and one page per `DataTypeKind` family, `Field` concern,
  or generic type, and that a leaf page never restates a generic rule. Drop "Generic enums and
  `Scalar` share `docs/generic.md`; no separate enums page", which this replaces. State the page
  shape and prose budget once, where the other layout rules live.
- Wherever it fixes public naming, add the Part 1 rule: a string naming a type spells the type.
- Fix the `TimeZone` / `Timezone` drift.

## Non-goals

- No behavior change beyond Parts 1 and 2. No new public API, no new capability.
- No new claim in a page. If a sentence cannot be traced to the inventories or to source, delete
  it rather than carry it over.
- No moving `arrow.md`, `playground.md`, `text.md`, or any other page not named here.
- No compatibility shim for a renamed method or an old enum tag.
- No marketing text, no reader-facing migration note, no benchmark-only page.

## Verification

Run what the touched surface requires, per `AGENTS.md`, and report exact results including
anything skipped:

```console
cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
python scripts/check_docs_examples.py
python -m mkdocs build --strict
grep -rn "default_pyvalue\|defaultJSValue\|default_pyhint\|defaultJSHint\|JSValueHint" . \
  --exclude-dir=target --exclude-dir=node_modules --exclude='*_PROMPT.md'
grep -rn '"io_mode"\|"data_type_id"\|"data_type_kind"\|"time_unit"\|"union_mode"\|"io_kind"\|"edge_algorithm"' \
  rust python node docs
grep -rn "datatype\.md\|field\.md\|generic\.md" docs README.md rust/README.md python/FIELDS.md scripts/
```

Plus the Python and Node test and boundary-benchmark suites for the bindings Part 2 touches. The
first two greps must return nothing; the third only intended `schema/...` and `generic/...`
paths. `--strict` fails on a broken link or dead anchor, so treat its output as the link check.

## Done when

- [ ] No public name spells a type in snake_case, and `EnumScalar` round-trips `"IOMode"` in all
      three languages and on the wire.
- [ ] `default_scalar` / `defaultScalar` is the only default accessor; the per-language
      projections and their machinery are deleted, and `Scalar.as_py` / `asJs` covers every
      family.
- [ ] `docs/schema/datatype/`, `docs/schema/field/`, and `docs/generic/` exist with the pages
      above; the three source pages are deleted.
- [ ] Every page is inside its budget, has one at-a-glance table, and every section has an
      example.
- [ ] No leaf page restates a generic rule; each links back instead.
- [ ] Every example passes the checker in all three languages, or is a marked Rust-only surface.
- [ ] Every measured table is verbatim or freshly measured, with its command and host sentence.
- [ ] `mkdocs.yml`, `docs/benchmarks.md`, `AGENTS.md`, both inventories,
      `scripts/docs_core_remaining.js`, and all inbound links updated; `mkdocs build --strict` is
      clean.
- [ ] Each new tree is smaller than the page it replaces.
