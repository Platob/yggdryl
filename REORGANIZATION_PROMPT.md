# Codebase reorganization brief

Reorganize the whole workspace around one rule:

> **A root `.rs` file is generic vocabulary — one trait, one enum, one shared
> value. A root folder is one abstraction layer. Inside a layer the same rule
> repeats: files are that layer's shared vocabulary, folders are its
> categories.**

`rust/src/generic/` (44 files, 21.8k lines) and `rust/src/io/` (14 files,
13.7k lines) are grab-bags that break the rule, and datatype/field/scalar code
for one type family is spread across three trees. This change moves every file
to the position the rule gives it, splits the monoliths it exposes, and mirrors
the same tree in tests, benchmarks, docs, and both extensions.

Follow `AGENTS.md`. This file contains only reorganization decisions; where it
contradicts an existing `AGENTS.md` layout rule, `AGENTS.md` is rewritten in
Phase 12 to match this file.

Three follow-on briefs depend on the tree this one creates and must land after
it, never interleaved — this brief's invariant is that the test count does not
move, and all three change behavior:

| Brief | Change |
| --- | --- |
| `SCALAR_HIERARCHY_PROMPT.md` | `Scalar` into family tiers; `DataType::Timestamp` to `DateTime64`; `Timezone` interned |
| `HIERARCHY_PROMPT.md` | the same three-floor rule for `DataType`, `Field`, `Holder`, `Media`, and the conformance sweep |
| `DOCUMENTATION_PROMPT.md` | every docs page rewritten to the new tree, with a use/narrow/optimize ladder in all three languages |

## Outcome

- Root holds the crate's contracts as one readable index: `IOBase`, `IOMedia`,
  the three storage roles, `Error`, `Metadata`, and every shared enum.
- Nine layer folders: `types`, `text`, `arrow`, `expression`, `uri`, `holder`,
  `coding`, `media`, `xxhash`.
- One type family lives in one folder: `types/<family>/{dtypes,fields,scalars,casts}.rs`.
- `generic/` and `io/` no longer exist.
- Two duplicate public `Coded<H>` types and two public `Text` types are resolved.
- `rust/tests/`, `rust/benchmarks/`, `python/src`, `python/yggdryl`,
  `python/tests`, `node/src`, `node/tests`, and `docs/` mirror the source tree.
- No behavior change. Every phase ends on a green build.

## Non-goals

- No API semantics change. Renames are path and spelling only, except the four
  recorded in **Renames**.
- No new abstraction, trait, generic parameter, or feature flag.
- No `pub use` facade that re-exports a module living somewhere else. A module
  owns its implementation; only `lib.rs` re-exports for the public prelude.
- No deprecated alias, shim, or transitional path. `AGENTS.md` forbids them.

## Read first

- `AGENTS.md` in full, especially *Workspace and ownership*, *Storage and I/O*,
  *Datatypes, fields, parsers, and errors*, *Documentation layout*.
- `rust/src/lib.rs`, `rust/src/generic/mod.rs`, `rust/src/io/mod.rs`.
- `rust/Cargo.toml` `[[bench]]` and `[[test]]` blocks.
- `mkdocs.yml` nav, `.api-inventory.txt`, `.api-bindings.txt`.

## Move rules

1. Every file move is `git mv`. A phase that moves files makes **no** logic
   edits beyond `use`/`mod`/path fixes and the module doc comment.
2. Splitting a monolith is its own commit, separate from the move that exposed
   it. Split by moving whole items; never retype a body.
3. Every new module starts with a `//!` line saying what it owns. The workspace
   lints `missing_docs = "warn"`; a layer `mod.rs` also names the layer and the
   layer below it.
4. A folder gets a `mod.rs` only when it is a layer or a category. A file's own
   parts use the sibling-directory form already used in this repo
   (`uri.rs` + `uri/pattern.rs`) — that directory is **not** a layer.
5. A file whose contents are *redistributed* by a phase ends under ~1200 lines.
   A file that only changes folder and `use` lines keeps its size; see
   **Monoliths** for the split list and what is deliberately left alone.
6. `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` pass at the end
   of every phase, under default features and under `--features "parquet iceberg"`.

## Target tree — root

Files. One trait or one enum each, named for the type it owns.

| File | Owns | From |
| --- | --- | --- |
| `error.rs` | `Error`, `Result` | unchanged |
| `metadata.rs` (+ `metadata/`) | `Metadata`, `ProtocolMetadata`, iterators, `sorted_pairs` | `metadata.rs`, `generic/pairs.rs` |
| `path.rs` | internal borrowed value path | unchanged |
| `iobase.rs` (+ `iobase/`) | `IOBase`, `delegate_iobase!`, `DEFAULT_STREAM_BATCH_SIZE` | `io/mod.rs` |
| `iomedia.rs` (+ `iomedia/`) | `IOMedia`, `delegate_iomedia!` | `io/media.rs` |
| `iofolder.rs` | `IOFolder` | `io/roles.rs` |
| `iofile.rs` | `IOFile` | `io/roles.rs` |
| `iopath.rs` | `IOPath` | `io/roles.rs` |
| `iocursor.rs` | `IOCursor`, `Cursor` | `io/cursor.rs` |
| `iokind.rs` | `IOKind` | `generic/io_kind.rs` |
| `iomode.rs` | `IOMode` | `generic/io_mode.rs` |
| `listing.rs` | `Listing` | `io/listing.rs` |
| `bytestream.rs` | `ByteStream` | `io/stream.rs` |
| `codec.rs` | `Codec`, `Encoder`, `Level` | `generic/codec.rs` |
| `digest.rs` | `Digest`, `DigestAlgorithm`, `DigestBytes`, `Digester` | `generic/digest.rs` |
| `datatype_id.rs` | `DataTypeId` | `generic/datatype_id.rs` |
| `datatype_kind.rs` | `DataTypeKind` | `generic/datatype_kind.rs` |
| `edge_algorithm.rs` | `EdgeAlgorithm` | `generic/edge_algorithm.rs` |
| `media_type.rs` | `MediaType` | `generic/media_type.rs` |
| `mime_type.rs` | `MimeType` | `generic/mime_type.rs` |
| `scheme.rs` | `Scheme` | `generic/scheme.rs` |
| `time_unit.rs` | `TimeUnit` | `generic/time_unit.rs` |
| `union_mode.rs` | `UnionMode` | `generic/union_mode.rs` |
| `timezone.rs` (+ `timezone/registry.rs`) | `Timezone` | `generic/timezone/` |
| `i256.rs` | `I256` | `generic/i256.rs` |

Naming: an `IO`-prefixed type takes the acronym unbroken (`iobase.rs`,
`iomedia.rs`, `iokind.rs`); every other file is snake_case of the type name.

Folders. One abstraction layer each.

| Folder | Layer |
| --- | --- |
| `types/` | the type system: datatypes, fields, scalars, casts |
| `text/` | structured text values and their formats |
| `arrow/` | Arrow runtime interop |
| `expression/` | predicates, projection, pushdown |
| `uri/` | resource identifiers |
| `holder/` | byte storage handles |
| `coding/` | content codings over a handle |
| `media/` | record encodings and the table format |
| `xxhash/` | the xxHash protocol implementation |

## Target tree — `types/`

Files own cross-family behavior. Folders own one type family. Adding a
datatype touches one folder plus the enum definition.

```
types/
  mod.rs            layer doc, family module list, re-exports for lib.rs
  dtype.rs          the DataType enum, validate, cross-family constructors
  field.rs          the Field struct, cache-aware mutation, metadata ownership
  scalar.rs         the Scalar enum, Eq/Ord/Hash, canonical Display
  typed.rs          TypedField<K>, TypedScalar<K>, define_field_types!
  parser.rs         recursive grammar dispatch for DataType and Field
  serde.rs          structural serialization for all three
  arrow.rs          Arrow schema/field conversion dispatch
  cast.rs           ArrowCast, the recursive array/batch planner
  budget.rs         materialization budget and buffer reservation
  value.rs          validate_value, canonicalize_value
  default.rs        default_value, preflight_schema
  diff.rs           Differences, show_diff
  merge.rs          widening and recode
  compatibility.rs  scheme compatibility projection
  vocabulary.rs     LOGICAL_NAMES and the logical-name registry
  arithmetic.rs     Arithmetic over Scalar
  enum_scalar.rs    EnumScalar
  pretty.rs         Pretty
  protocol/         ProtocolField, ProtocolFieldMut, http.rs
  boolean/          Null, Boolean
  integer/          Int8..Int64, UInt8..UInt64
  floating/         Float16, Float32, Float64
  decimal/          Decimal32/64/128/256
  temporal/         Timestamp, Date32/64, Time32/64, Duration32/64, Interval
  text/             Utf8, LargeUtf8, Utf8View
  ascii/            Ascii32/64/128, AsciiDictionary, iso.rs
  binary/           Binary, FixedSizeBinary, LargeBinary, BinaryView
  nested/           List, ListView, FixedSizeList, LargeList, LargeListView, Struct, Union, Variant, Dictionary, Map, RunEndEncoded
  geospatial/       Geometry, Geography, wkb.rs
```

Every family folder holds exactly this shape; a file is omitted only when the
family genuinely has none of that behavior.

| File | Owns |
| --- | --- |
| `mod.rs` | family doc, the variant list this folder covers |
| `dtypes.rs` | `impl DataType` for this family: predicates, validated constructors, unit/precision rules |
| `fields.rs` | the sealed field markers (`define_field_types!`) and family `TypedField` behavior |
| `scalars.rs` | the `Scalar` arms, family typed scalars, conversions, arithmetic arms |
| `casts.rs` | the family's arms of the Arrow cast planner and array ingest/render |
| `parser.rs` | family grammar, only where the family has its own spellings |
| `tests.rs` | family unit tests |

The three names in a folder always agree: `<F>Type` in `dtypes.rs`, `<F>` in
`scalars.rs`, `<Member>Type` markers in `fields.rs`.

`Null` rides with `Boolean` in `types/boolean/`: both are parameterless, so
neither earns a family enum and neither would fill a folder. `text/` and
`ascii/` are separate families because `Ascii32/64/128` carry a fixed width, an
extension identity, and a dictionary that UTF-8 does not — see the *Symmetry
law* in `SCALAR_HIERARCHY_PROMPT.md`, which requires each of these ten folders
to own a matching datatype enum, marker set, and value enum.

Fix while moving: `datatype/floating.rs` today holds the **decimal**
constructors. They go to `types/decimal/dtypes.rs`. `types/floating/dtypes.rs`
holds only Float16/32/64.

## Target tree — `holder/`

```
holder/
  mod.rs        the Holder enum over every concrete IOBase implementation
  buffer.rs     Buffer
  local/        path.rs folder.rs file.rs tests.rs
  arrowfs/      path.rs folder.rs file.rs system.rs tests.rs
  buffered/     mod.rs options.rs page.rs tests.rs
  tests.rs
```

`IOBase`, `IOMedia`, `IOFolder`, `IOFile`, `IOPath`, `IOCursor`, `Listing`, and
`ByteStream` are root vocabulary, not part of this layer. A backend is a folder
here supplying the three roles; nothing else in the tree changes to add one.

## Target tree — `coding/`

```
coding/
  mod.rs      Coding<H>, the transparent wrapping handle
  coded.rs    Coded, the enum over Gzip/Zlib/Zstd handles
  gzip.rs     (+ gzip/tests.rs)   load, dump, reader, writer
  zlib.rs     (+ zlib/tests.rs)
  zstd.rs     (+ zstd/tests.rs)
  tests.rs
```

`Codec` stays root vocabulary and remains the only dispatcher.

## Target tree — `media/`

```
media/
  mod.rs        the Media enum over every concrete IOMedia implementation
  options.rs    RecordOptions, IORecordOptions, CommitBuffer, WriteLimitState, DEFAULT_ROOT_NAME
  inference.rs  format/media inference
  magic.rs      content sniffing, MAGIC_PROBE_LEN
  merge.rs      (+ merge/tests.rs)      row merge on a match key
  partition.rs  (+ partition/tests.rs)  Hive partition routing, partition_text, NULL_PARTITION
  ipc/          Arrow IPC
  parquet/      Apache Parquet (feature-gated)
  avro/         Apache Avro
  text/         Text<H>, TextOptions, LineSep, line splitting and rendering
  iceberg/      the table format over those encodings (feature-gated)
  tests.rs
```

`media/text/` is today's `text/line/`. The structured-text *value* layer stays
in `text/`; only the row media moves here.

The no-feature Iceberg fallback in `lib.rs` becomes
`#[path = "media/iceberg/types.rs"]` under `media/mod.rs`.

## Target tree — `text/` and `uri/`

```
text/
  mod.rs        layer doc and shared entry points
  codec.rs      TextCodec, Json, Jsonl, Toml, Yaml, Limited markers
  structured.rs Structured, the runtime enum over TextCodec implementations
  format.rs formatting.rs display.rs limits.rs loading.rs io.rs wire.rs
  position.rs typed.rs placeholder.rs
  json/         mod.rs parser.rs wire.rs tests.rs
  yaml/         mod.rs parser.rs tests.rs
  toml/         mod.rs parser.rs wire.rs tests.rs
```

```
uri/
  mod.rs        Uri and the shared component model
  url.rs        Url
  urn.rs        Urn
  authority.rs  Authority, credentials, S3 host inference
  path.rs       UriPath, PathSegments, Parents, UriParents, UrlParents
  extensions.rs Extensions and suffix handling
  parser.rs     the one component parser, percent escapes, byte offsets
  glob.rs       is_glob, glob_parts, matches_glob
  hive.rs       hive_partitions, hive_partitions_under, children_where
  pattern.rs    (+ pattern/tests.rs)
  tests.rs
```

`uri.rs` is 3145 lines today; the split above is the reason this layer becomes
a folder rather than a root file.

## Renames

Four public spellings change, because two pairs collide today.

| Was | Becomes | Why |
| --- | --- | --- |
| `io::Coded<H>` (struct) | `coding::Coding<H>` | it is the wrapper that *applies* a coding |
| `generic::Coded<H>` (enum) | `coding::Coded` | parallel to `Holder` and `Media`: the enum over concrete handles |
| `generic::Text` (enum) | `text::Structured` | frees `Text` for the row media |
| `text::line::Text<H>` | `media::text::Text<H>` | same type name, media layer |

`Gzip`, `Zlib`, and `Zstd` remain aliases of the wrapper with the codec fixed,
now over `Coding<H>`.

No other public name changes. Every other difference a caller sees is a module
path, listed in the Phase 12 inventory update.

## Monoliths

Split by this change, because their contents are redistributed:

| File | Lines | Phase | Goes to |
| --- | --- | --- | --- |
| `field/cast/plan.rs` | 5936 | 8 | `types/cast.rs`, `types/budget.rs`, six `types/<family>/casts.rs` |
| `io/tests.rs` | 3838 | 10 | `tests/holder/`, `tests/media/`, `iobase/tests.rs` |
| `uri.rs` | 3145 | 9 | the `uri/` layer |
| `io/mod.rs` | 3050 | 1 | `iobase.rs` + `iobase/` parts |
| `generic/scalar.rs` | 2499 | 7 | `types/scalar.rs`, `types/{floating,integer}/scalars.rs` |
| `field/mod.rs` | 2203 | 6 | `types/field.rs`, `types/typed.rs`, family `fields.rs` |
| `datatype/parser.rs` | 1671 | 5 | `types/parser.rs` + family `parser.rs` |
| `datatype/tests.rs` | 1645 | 10 | `tests/types/` per family |
| `generic/arithmetic.rs` | 1630 | 7 | `types/arithmetic.rs` + family `scalars.rs` arms |
| `generic/options.rs` | 1614 | 4 | `media/options.rs` + `options/{limits,commit}.rs` |
| `metadata.rs` | 1503 | 1 | `metadata.rs` + `metadata/` parts |
| `datatype/nested.rs` | 1443 | 5 | `types/nested/dtypes.rs` + `types/nested/fields.rs` |
| `field/value.rs` | 1319 | 6 | `types/value.rs` + family `scalars.rs` validation arms |
| `generic/mime_type.rs` | 1290 | 1 | `mime_type.rs` + `mime_type/registry.rs` |
| `io/partition/tests.rs` | 1163 | 10 | `tests/media/partition.rs` |
| `datatype/serde.rs` | 1130 | 5 | `types/serde.rs` + family arms |
| `field/diff.rs` | 1102 | 6 | `types/diff.rs` |
| `generic/wkb.rs` | 1101 | 7 | `types/geospatial/wkb.rs` + `wkb/{read,write}.rs` |

Deliberately **not** split. These move folder and nothing else; their size is a
codec's own complexity, not a layout problem, and splitting them here would
hide the reorganization inside unrelated churn:

`iceberg/{tests,manifest,table,metadata,scan}.rs`, `avro/{tests,batch,container}.rs`,
`arrow/{value,mod}.rs`, `expression/{parser,mod,bind}.rs`, `parquet/{tests,mod}.rs`,
`xxhash/tests.rs`, `arrowfs/tests.rs`.

## Phases

Each phase is one commit (a split inside a phase is its own commit), ends green,
and touches only what it names.

### Phase 0 — baseline

Record, in the commit message, the exact output of:

```
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path rust/Cargo.toml --workspace --all-targets --features "parquet iceberg" -- -D warnings
cargo test  --locked --manifest-path rust/Cargo.toml --workspace --all-targets --features "parquet iceberg"
cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --no-default-features --lib
cargo doc   --locked --manifest-path rust/Cargo.toml --workspace --no-deps --features "parquet iceberg"
```

Test count and warning count are the invariant for every later phase.

### Phase 1 — root vocabulary

Hoist to root files per the root table: every `generic/` enum, plus `IOBase`,
`IOMedia`, the three roles, `IOCursor`, `Listing`, `ByteStream`. Split
`io/roles.rs` into three files and `generic/timezone/mod.rs` into
`timezone.rs` + `timezone/registry.rs`.

After this phase `generic/` holds only `scalar`, `holder`, `media`, `options`,
`inference`, `magic`, `coded`, `typed`, `arithmetic`, `decimal`, `temporal`,
`enum_scalar`, `wkb`, `iso`, `pairs`; `io/` holds only `buffer`, `coding`,
`merge`, `partition`.

Split `iobase.rs` if it lands over 1200 lines: `iobase/bytes.rs`,
`iobase/hierarchy.rs`, `iobase/lifecycle.rs`, `iobase/transfer.rs`.

### Phase 2 — `coding/`

Move `gzip/`, `zlib/`, `zstd/`, `io/coding.rs`, `generic/coded.rs`. Apply the
two `Coded` renames. Update `Codec` dispatch call sites.

### Phase 3 — `holder/`

Move `generic/holder.rs` → `holder/mod.rs`, `io/buffer.rs`, `local/`,
`arrowfs/`, `buffered/`.

### Phase 4 — `media/`

Move `generic/media.rs` → `media/mod.rs`, `generic/options.rs`,
`generic/inference.rs`, `generic/magic.rs`, `io/merge*`, `io/partition*`,
`ipc/`, `parquet/`, `avro/`, `text/line/` → `media/text/`, `iceberg/`.
Split `media/options.rs` (1614 lines) into `options.rs` + `options/limits.rs` +
`options/commit.rs`.

### Phase 5 — `types/`, part 1: datatypes

Create `types/`. Move `datatype/mod.rs` → `types/dtype.rs` and distribute the
family predicates and constructors into `types/<family>/dtypes.rs`. Move
`datatype/nested.rs` → `types/nested/dtypes.rs`, `datatype/ascii.rs` →
`types/ascii/{dtypes.rs,dictionary.rs}`, `datatype/geospatial.rs` →
`types/geospatial/dtypes.rs`, and the decimal constructors out of
`datatype/floating.rs` into `types/decimal/dtypes.rs`. Cross-family files
(`parser`, `serde`, `arrow`, `merge`, `compatibility`, `default`,
`vocabulary`, `logical`) move to `types/` root files; `datatype/comparison.rs`
folds into `types/diff.rs`.

### Phase 6 — `types/`, part 2: fields

Move `field/mod.rs` → `types/field.rs`, `field/typed.rs` → `types/typed.rs`,
`field/protocol/` → `types/protocol/`, and each family marker file
(`field/integer.rs`, …) → `types/<family>/fields.rs`. `field/scalar.rs` →
`types/boolean/fields.rs`. `field/value.rs`, `field/diff.rs`,
`field/pretty.rs`, `field/parser.rs`, `field/serde.rs`, `field/arrow.rs` merge
into their `types/` root counterparts.

### Phase 7 — `types/`, part 3: scalars

Move `generic/scalar.rs` (2499) → `types/scalar.rs` for the enum, `Eq`/`Ord`/
`Hash`/serde and `Display`; `Float16/32/64` and `Float` → `types/floating/scalars.rs`;
`Integer` → `types/integer/scalars.rs`. Distribute `generic/typed.rs` markers
into each `types/<family>/scalars.rs`, keeping the macro in `types/typed.rs`.
`generic/temporal.rs` → `types/temporal/scalars.rs`, `generic/decimal.rs` →
`types/decimal/scalars.rs`, `generic/wkb.rs` → `types/geospatial/wkb.rs`,
`generic/iso.rs` → `types/ascii/iso.rs`, `generic/enum_scalar.rs` and
`generic/arithmetic.rs` → `types/` root files with family arms in each
`scalars.rs`.

### Phase 8 — `types/`, part 4: casts

Split `field/cast/plan.rs` (5936 lines, the largest file in the crate):

| Destination | Content |
| --- | --- |
| `types/cast.rs` | `ArrowCast`, `ArrayCastPlan`, plan kinds, `cast_record_batch`, `cast_field_array` |
| `types/budget.rs` | every `reserve_*`, `MaterializationBudget`, scratch vectors, `CountingWriter` |
| `types/nested/casts.rs` | dictionary, run-end, union, list, struct planning; exposure and logical-null walks |
| `types/temporal/casts.rs` | `is_temporal_arrow`, `render_temporal_text`, `ingest_temporal_text`, `holds_temporal` |
| `types/ascii/casts.rs` | `ingest_ascii_array`, `padded_ascii_array`, `render_ascii_text`, `ascii_cell` |
| `types/geospatial/casts.rs` | `validate_wkb_ingest`, `render_wkt_array`, `wkt_for_cell` |
| `types/decimal/casts.rs` | `DecimalText` |
| `types/binary/casts.rs` | byte-array pointer equality, view buffers, projected byte length |

Then delete `datatype/`, `field/`, `generic/`, and `io/`. They must already be
empty; a leftover file means an earlier phase was incomplete.

### Phase 9 — `text/` and `uri/`

Move `json/`, `yaml/`, `toml/` under `text/`; `generic/text.rs` →
`text/structured.rs` with the `Structured` rename. Split `uri.rs` into the
`uri/` layer above.

### Phase 10 — Rust tests and benchmarks

`rust/tests/` mirrors the source tree; each layer is one integration binary
plus a module folder.

| Binary | Modules |
| --- | --- |
| `tests/types.rs` | `types/{dtype,field,scalar,parser,serde,cast,diff,default,protocol,boolean,integer,floating,decimal,ascii,binary,temporal,nested,geospatial}.rs` |
| `tests/holder.rs` | `holder/{buffer,local,arrowfs,buffered,roles}.rs` |
| `tests/media.rs` | `media/{options,ipc,parquet,avro,text,iceberg,merge,partition}.rs` |
| `tests/coding.rs` | `coding/{gzip,zlib,zstd,coded}.rs` |
| `tests/text.rs` | `text/{format,placeholder,value,json,yaml,toml,structured}.rs` |
| `tests/uri.rs` | `uri/{parser,url,urn,glob,hive,pattern}.rs` |
| `tests/arrow.rs` | `arrow/{value,rows,batch,cast_coverage}.rs` |
| `tests/expression.rs`, `tests/xxhash.rs` | as today |
| `tests/allocations.rs`, `tests/docs_index.rs`, `tests/interop.rs` | cross-cutting; keep as separate binaries |

Fold today's loose binaries into the mirror: `batch_cast`, `cast_coverage`,
`default_scalar`, `row_value`, `value_bounds`, `combined` → `tests/types.rs`
and `tests/arrow.rs`; `enums` → `tests/types.rs` and the root-vocabulary
module; `avro_interop` + `iceberg_interop` → `tests/interop.rs`.

Split `io/tests.rs` (3838) and `datatype/tests.rs` (1645) into the per-layer
and per-family files above. In-source `mod tests` blocks stay in-source and
move with their module.

`rust/benchmarks/` mirrors the same names: `types.rs`, `holder.rs`, `media.rs`,
`coding.rs`, `text.rs`, `uri.rs`, `expression.rs`, `xxhash.rs`, each with a
folder of the same modules. `benchmarks/datatype/` and `benchmarks/field/`
merge into `benchmarks/types/`; `benchmarks/io/` and `benchmarks/arrowfs/` into
`benchmarks/holder/` and `benchmarks/media/`; `benchmarks/{json,yaml,toml}.rs`
into `benchmarks/text/`. Rewrite every `[[bench]]` and `[[test]]` block in
`rust/Cargo.toml`, keeping the existing `required-features` gates.

### Phase 11 — extensions

Both crates mirror the layer names; neither implements core logic.

```
python/src/{lib.rs, iobase.rs, iomedia.rs, types/, holder/, coding/, media/, text/, uri.rs, expression.rs, xxhash.rs, enums.rs}
python/yggdryl/{__init__.py, types/, holder/, coding/, media/, text/, uri.py, expression.py, xxhash.py, enums/}
python/tests/{types/, holder/, coding/, media/, text/, uri/, test_*.py at the layer they cover}
python/benchmarks/{types.py, holder.py, coding.py, media.py, text.py, uri.py, xxhash.py}
node/src/{lib.rs, iobase.rs, iomedia.rs, types/, holder/, coding/, media/, text/, uri.rs, expression.rs, xxhash.rs, enums.rs}
node/tests/{types/, holder/, coding/, media/, text/, uri/}  each keeping its .test.js + .types.ts pair
node/benchmarks/{types.js, holder.js, coding.js, media.js, text.js, xxhash.js}
```

Python public module paths follow the layer names; `python/yggdryl/fields/`
becomes `python/yggdryl/types/`, `python/yggdryl/enums/` stays (it is the root
vocabulary view). Node keeps its single entry point and regroups only files.
Keep every `.pyi` beside its module and every `.types.ts` beside its test.

### Phase 12 — contracts and documentation

- Rewrite the `AGENTS.md` layout rules that name the old tree. At minimum:
  *Workspace and ownership* bullets 3, 5, 6, 7, 8, 9, 10, 11, 12, 13;
  *Storage and I/O* `IOBase`/`Buffered` bullets; *Arrow and allocation*
  `yggdryl::arrow` bullet. State the root-file / layer-folder rule once, at the
  top of *Workspace and ownership*, and delete every rule it subsumes.
- `mkdocs.yml` nav becomes the layer list. One page per layer, with the layer's
  categories as sections: `types.md` (families as sections), `holder.md`
  (local, arrowfs, buffered), `coding.md` (gzip, zlib, zstd), `media.md`
  (ipc, parquet, avro, text, iceberg), `text.md` (json, yaml, toml), `uri.md`,
  `arrow.md`, `expression.md`, `xxhash.md`. Retire `datatype.md`, `field.md`,
  `generic.md`, `io.md`, `local.md`, `arrowfs.md`, `buffered.md`, `gzip.md`,
  `zlib.md`, `zstd.md`, `ipc.md`, `parquet.md`, `avro.md`, `iceberg.md`,
  `json.md`, `yaml.md`, `toml.md` by folding their content into the layer page
  they belong to — content moves, it is not deleted.
- Regroup `.api-inventory.txt` and `.api-bindings.txt` under the new module
  headings, in tree order. Every moved public item keeps its entry with the new
  path; the four renames replace their entries.
- Update `docs/architecture.md` to describe the nine layers and the root rule.
- Update every intra-doc link and every `use yggdryl::…` line in doc examples;
  `scripts/check_docs_examples.py` and `python -m mkdocs build --strict` must
  both pass.
- Update `README.md` module list and `docs/index.md`.

## Verification

Per phase:

```
cargo fmt --manifest-path rust/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path rust/Cargo.toml --workspace --all-targets --features "parquet iceberg" -- -D warnings
cargo test  --locked --manifest-path rust/Cargo.toml --workspace --all-targets --features "parquet iceberg"
cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --no-default-features --lib
```

Before handoff, additionally:

```
cargo doc   --locked --manifest-path rust/Cargo.toml --workspace --no-deps --features "parquet iceberg"
cargo bench --locked --manifest-path rust/Cargo.toml --benches --no-run --features "parquet iceberg"
python -m pytest python/tests
npm test --prefix node && npm run --prefix node test:package
python scripts/check_docs_examples.py
python -m mkdocs build --strict
```

Invariants:

- Test count matches the Phase 0 baseline exactly. A phase that changes it is
  wrong: a move never adds or removes a test.
- Warning count stays zero.
- `--no-default-features --lib` still compiles: `types/*/casts.rs`,
  `media/`, and `arrow/` stay behind the `arrow` gate; `types/*/dtypes.rs`,
  `fields.rs`, and `scalars.rs` do not.
- `git log --follow` resolves for every moved file. Use `git mv`, never
  delete-and-create.

## Completion

- `rust/src/generic/` and `rust/src/io/` do not exist.
- `rg -n "crate::generic|crate::io::|yggdryl::generic|yggdryl::io" rust python node docs` returns nothing.
- Every root `.rs` file owns exactly one trait, enum, or shared value.
- Every root folder has a `mod.rs` whose first line names the layer.
- Every `types/<family>/` folder has `dtypes.rs`, `fields.rs`, `scalars.rs`,
  and `casts.rs` unless the family provably has none of that behavior, and the
  omission is stated in that folder's `mod.rs`.
- Every file in the **Monoliths** split list is under 1200 lines, and the
  deferred list is unchanged in content.
- `rust/tests/`, `rust/benchmarks/`, `python/`, `node/`, and `docs/` name the
  same layers as `rust/src/`.
- `AGENTS.md`, `mkdocs.yml`, `.api-inventory.txt`, and `.api-bindings.txt`
  describe the new tree and nothing of the old one.
