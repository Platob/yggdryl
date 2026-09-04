# Documentation rewrite brief

Rewrite every documentation page to mirror the reorganized tree, with the
shortest description text that still changes a decision, and an example ladder
that runs from the simplest correct call to the optimized one — in Rust,
Python, and JavaScript.

Follow `AGENTS.md`, *Optimize documentation for lookup* and *Documentation
layout*. Land `REORGANIZATION_PROMPT.md`, `SCALAR_HIERARCHY_PROMPT.md`, and
`HIERARCHY_PROMPT.md` first: this brief documents the tree they produce.

## Outcome

- `mkdocs.yml` nav is the layer list; every page maps to one module in the
  tree, at the same depth.
- Every page follows one skeleton, so a reader who learns one page can navigate
  all of them.
- Every concept is demonstrated at three rungs — use, narrow, optimize — with
  Rust, Python, and JavaScript tabs, and Rust-only rungs labeled.
- Every code block is self-contained, asserts, and passes
  `scripts/check_docs_examples.py`.
- `python -m mkdocs build --strict` passes with no dead link.

## Nav

Sections are layers; pages are the layer's own modules.

```yaml
nav:
  - Home:
      - index.md
      - Architecture: architecture.md
      - Testing: testing.md
      - Benchmarks: benchmarks.md
      - Contributing: contributing.md
  - Getting started: getting-started.md
  - Vocabulary:
      - overview: vocabulary/index.md          # the root enums and traits
      - enums: vocabulary/enums.md             # Codec, Scheme, MimeType, MediaType, TimeUnit, Timezone, IOKind, IOMode, DataTypeId, DataTypeKind, UnionMode, EdgeAlgorithm
      - contracts: vocabulary/contracts.md     # IOBase, IOMedia, IOFolder, IOFile, IOPath, IOCursor
      - errors: vocabulary/errors.md
      - metadata: vocabulary/metadata.md
  - Types:
      - overview: types/index.md               # the three floors, one diagram, one ladder
      - datatype: types/datatype.md
      - field: types/field.md
      - scalar: types/scalar.md
      - cast: types/cast.md
      - boolean: types/boolean.md
      - integer: types/integer.md
      - floating: types/floating.md
      - decimal: types/decimal.md
      - temporal: types/temporal.md
      - ascii: types/ascii.md
      - binary: types/binary.md
      - nested: types/nested.md
      - geospatial: types/geospatial.md
      - protocol: types/protocol.md
  - Storage:
      - overview: holder/index.md
      - holder: holder/holder.md
      - local: holder/local.md
      - arrowfs: holder/arrowfs.md
      - buffered: holder/buffered.md
  - Coding:
      - overview: coding/index.md
      - gzip: coding/gzip.md
      - zlib: coding/zlib.md
      - zstd: coding/zstd.md
  - Media:
      - overview: media/index.md
      - options: media/options.md
      - ipc: media/ipc.md
      - parquet: media/parquet.md
      - avro: media/avro.md
      - text: media/text.md
      - iceberg: media/iceberg.md
      - partition: media/partition.md
  - Text:
      - overview: text/index.md
      - json: text/json.md
      - yaml: text/yaml.md
      - toml: text/toml.md
  - Identifiers:
      - uri: uri.md
  - Runtime:
      - arrow: arrow.md
      - expression: expression.md
      - xxhash: xxhash.md
      - playground: playground.md
  - Extensions:
      - Python: extensions/python.md
      - JavaScript: extensions/javascript.md
```

Content moves, it is not deleted. Every retired page's material lands on the
page that now owns that module; nothing is dropped without a line in the commit
message saying what and why.

## Page skeleton

Every page, without exception:

```markdown
# <module name>

<one sentence: what this module owns and nothing else>

## Contract

<the smallest table or list that pins the behavior: what is validated, what is
lazy, what is cached, what errors. No prose restating signatures.>

## Use

<rung 1 — three tabs>

## Narrow

<rung 2 — three tabs>

## Optimize

<rung 3 — three tabs, or one Rust tab plus an explicit Rust-only line>

## Edges

<only the non-obvious ones: what is rejected and with which error, what a null
does, what an empty value does, what overflows. One line each.>

## Performance

<the generated benchmark table for this module, or nothing. Never a
benchmark-only section with no numbers in it.>
```

An overview page (`*/index.md`) replaces *Use/Narrow/Optimize* with one ladder
over the layer as a whole and a table linking each module to its page.

## The example ladder

Three rungs, always in this order, and each rung is a complete runnable program
with assertions — never a fragment of the rung above.

| Rung | Question it answers | Shape |
| --- | --- | --- |
| **Use** | how do I do the obvious thing? | one value, one call, one assertion. No options, no generics, no markers. |
| **Narrow** | how do I say what I already know? | the drill-down: a family enum, a typed marker, a capability trait. Shows what the type system buys. |
| **Optimize** | how do I do it without paying? | streaming instead of collecting, pushdown instead of filtering, a borrowed `as_*` instead of `into_*`, a `Copy` leaf instead of `Scalar`, a reused options value instead of a rebuilt one. |

Worked example of the ladder on `types/temporal.md`:

```markdown
## Use

=== "Rust"
    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    let dtype = DataType::from_str("datetime64(us, UTC)")?;
    let at = Scalar::from_datetime(1_700_000_000_000_000, TimeUnit::Microsecond, Timezone::UTC)?;
    assert_eq!(at.dtype()?, dtype);
    ```
=== "Python" …
=== "JavaScript" …

## Narrow

=== "Rust"
    ```rust
    use yggdryl::types::temporal::{DateTime64, TemporalScalar, TemporalValue};

    // Drill from the value to its family to its leaf; no match on 30 variants.
    let Scalar::Temporal(TemporalScalar::DateTime64(at)) = value else { unreachable!() };
    assert_eq!(at.unit(), TimeUnit::Microsecond);      // 16 bytes, Copy
    ```
=== "Python" …
=== "JavaScript" …

## Optimize

=== "Rust"
    ```rust
    // Generic over the family: monomorphized, no dispatch, no allocation.
    fn floor<T: TemporalValue>(value: T, unit: TimeUnit) -> yggdryl::Result<T> {
        value.with_unit(unit)
    }
    ```

The generic form is Rust-only. Python and JavaScript reach the same behavior
through `value.with_unit(unit)` on the dynamic value.
```

Rules for the ladder:

- Every rung compiles and asserts. `scripts/check_docs_examples.py` runs them
  all; a block that cannot run is marked with valid superfence `ignore` syntax
  and is reported, never silently skipped.
- Tabs are always Rust, Python, JavaScript, in that order, expressing the same
  operation idiomatically — not a transliteration.
- A rung that has no binding says so in one line under the tab. Never invent an
  API to fill a tab, and never omit the tab silently.
- No rung repeats another rung's setup as prose. If the setup is long, the rung
  is the wrong size.

## Writing rules

Enforce, per page:

- One H1, one purpose sentence. The sentence names what the module owns, not
  what it is "for".
- Every sentence changes a decision. Delete anything that restates a signature,
  recaps a previous section, praises the design, or explains why an obvious
  thing is obvious.
- Contract before example, example before edge case, edge case before numbers.
- One fact lives in one place. Link, never paraphrase. A fact that belongs to
  two pages belongs to the lower one, and the higher one links down.
- Tables only for exact mappings — variant to Arrow type, spelling to
  datatype, error to condition. Never a table of prose.
- Name every type by its canonical spelling on first use in a section, then use
  it bare.
- No page describes another layer's behavior. `media/parquet.md` does not
  explain `IOBase`; it links to `vocabulary/contracts.md`.

Target: no page section over ~120 words of prose outside its code blocks. If a
section needs more, it is two sections or it belongs on another page.

## Renames to sweep

The reorganization and hierarchy briefs change spellings the docs use
throughout. Sweep every page, every example, and every prose mention:

| Old | New |
| --- | --- |
| `timestamp` (canonical spelling) | `datetime64` |
| `DataType::Timestamp` | `DataType::DateTime64` |
| `yggdryl::io::…` | `yggdryl::iobase`, `yggdryl::holder::…` |
| `yggdryl::generic::…` | root vocabulary, `types::…`, `media::…` |
| `yggdryl::datatype`, `yggdryl::field` | `yggdryl::types` |
| `io::Coded` | `coding::Coding` |
| `generic::Coded` | `coding::Coded` |
| `generic::Text` | `text::Structured` |
| `TemporalRef` | `&Temporal` |
| `field::temporal::Timestamp` | `types::temporal::DateTime64Type` |
| `TimestampScalar` | `DateTime64Scalar` |
| `EnumScalar` | `Enum` |
| `Xxh3_64` / `Xxh3_128` (Rust, TS classes, `Js*` aliases) | `Xxh3` / `Xxh128` |
| `xxhash.xxh3_64(…)` / `xxhash.xxh3_128(…)` (Python, JS) | `xxhash.xxh3(…)` / `xxhash.xxh128(…)` |

The `*Scalar` and `*Field` builder aliases stay. Per the naming law in
`SCALAR_HIERARCHY_PROMPT.md`, a bare name is the value and the suffix is the
`TypedScalar` / `TypedField` pairing, so a page shows `DateTime64` when it
means the value and `DateTime64Scalar` only when it means the builder. Do not
mix them in one example.

`timestamp` stays in exactly one place: the table of accepted foreign spellings
on `types/datatype.md`, where it is listed beside the SQL, Hive, and Spark
forms and shown displaying as `datetime64`.

## Generated content

- Benchmark tables come from release runs and name machine, runtime, and build.
  A page with no current numbers has no *Performance* section — never a
  placeholder.
- `docs/assets/` playground manifest is regenerated by
  `node scripts/build_docs_playground.js`; the CI drift check must pass.
- `rust/tests/docs_index.rs` compiles the landing-page example. Update it with
  `docs/index.md`, in the same commit.

## Phases

| # | Phase | Content |
| --- | --- | --- |
| 1 | nav and skeletons | `mkdocs.yml`, every page created with H1, purpose sentence, and empty section headings; strict build green with no dead link |
| 2 | content move | existing prose and examples relocated to the page that now owns them; nothing rewritten yet; nothing lost |
| 3 | rename sweep | the table above, across every page and every example; `check_docs_examples.py` green |
| 4 | ladders | *Use / Narrow / Optimize* written per page, three tabs each, all asserting |
| 5 | compression | the writing rules applied; every section cut to what changes a decision |
| 6 | overviews | the eight `*/index.md` layer pages, each with one ladder and one link table |
| 7 | generated | benchmark tables, playground manifest, `docs_index.rs`, README, `docs/architecture.md` |

Phases 1-3 are mechanical and land fast. Phase 4 is the work.

## Verification

```
python scripts/check_docs_examples.py
python -m mkdocs build --strict
node scripts/build_docs_playground.js --check
cargo test --locked --manifest-path rust/Cargo.toml --test docs_index --features "parquet iceberg"
```

Sweeps, each reported with its result:

- Every page has exactly one H1 and every required section heading.
- Every `=== "Rust"` block has a matching `=== "Python"` and
  `=== "JavaScript"` block, or an explicit Rust-only line.
- No page references a module path that no longer exists:
  `rg -n "yggdryl::(generic|io|datatype|field)\b" docs` returns nothing.
- No example spells an underscored type name: `rg -n "Xxh3_" docs` returns
  nothing, and the digest wire strings `xxh3-64` / `xxh3-128` appear only where
  a canonical digest string is being shown.
- Every public item in `.api-inventory.txt` appears on exactly one page, or is
  listed in that page's *Contract* table as intentionally undocumented.
- No prose section exceeds ~120 words.

## Completion

- The nav mirrors `rust/src/` at the same depth, section for layer, page for
  module.
- Every page follows the skeleton; every concept has all three rungs.
- Every code block runs and asserts; Rust-only rungs are labeled, never blank.
- No retired module path, no `timestamp` outside the foreign-spelling table.
- `mkdocs build --strict` and `check_docs_examples.py` both pass clean.
