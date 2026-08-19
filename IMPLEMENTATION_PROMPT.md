# Prompt: `doris` — Apache Doris 4.1 as a first-class target, and as the outside implementation that checks Parquet and Iceberg

Implement `rust/src/doris/`: the module that lets Yggdryl speak Apache Doris
4.1 - its type system, its DDL, its Stream Load wire, its export layout, its
table-value functions, and its Iceberg catalog - and then **use Doris as the
outside implementation** that proves the Parquet and Iceberg read/write
protocols this workspace already ships are correct against an engine nobody
here wrote. Today the workspace validates Iceberg against PyIceberg and Avro
against fastavro; Parquet is checked against PyArrow, which is the same
Rust/C++ lineage the `parquet` crate came from. Doris is a genuinely
independent C++ reader and writer with its own Parquet and Iceberg
implementation, so a round trip through it is the strongest correctness signal
available: if Doris reads every row and every nested value of what
`rust/src/parquet/` and `rust/src/iceberg/` wrote, and Yggdryl reads back every
row of what Doris wrote, both protocols are settled.

Deliver it complete: fully implemented, edge-case tested on **every** Doris
type including the deeply nested ones, benchmarked against implementations the
reader already trusts, interop-checked both directions against a real Doris,
and documented with running examples.

Work on branch `claude/doris-4-1-impl`; commit and push there.

---

## 0. Read first (non-negotiable)

1. **`AGENTS.md`, in full.** It is the real spec. The sections that govern this
   task: *Order of work* (line 9), *Source layout and scope* (17), *Storage and
   I/O contract* (109), *Media implementation standard* (183), *Table format
   contract* (228), *Documentation organization* (353), *Exact method
   vocabulary* (409), *Error message contract* (578), *Native value behavior*
   (604), *Parser contract* (626), *Arrow and allocation contract* (707),
   *Binding boundary contract* (841), *Python extension* (874), *JavaScript
   extension* (1064), *Required checks* (1128).
2. `rust/src/iceberg/` — the model for this module's shape. One folder, one
   file per concern (`types.rs`, `schema.rs`, `partition.rs`, `metadata.rs`,
   `options.rs`, `catalog.rs`, `scan.rs`, `table.rs`), a non-default feature,
   no dependency for the format itself, and `iceberg::IcebergOptions` as the
   single place every knob is resolved (explicit → table property → default).
   `doris/` is built exactly this way. Read `iceberg/options.rs` before writing
   `doris/options.rs`.
3. `rust/src/parquet/` and `rust/src/ipc/` — the two record encodings Doris
   will read and write. The three record methods
   (`read_arrow_batch_reader`, `write_arrow_batch_reader`,
   `append_arrow_batch_reader`) are the only decode and encode entry points; a
   Stream Load body is produced *through* them, never by a private writer.
4. `rust/src/expression/` — `Expression`, `Bound`, `pushdown.rs`. The predicate
   Doris receives in a TVF `WHERE` clause is this expression rendered to Doris
   SQL, and the residual split is the one already implemented. There is no
   second filter representation.
5. `rust/src/datatype/compatibility.rs` (`to_scheme_compat`, line 112) and
   `rust/src/enums/scheme.rs` (`COMPATIBILITY_TARGETS`, line 91). Doris becomes
   the sixth compatibility target; the walker is the one generic recursive
   walker with a per-target scalar matrix - never a fork.
6. `rust/src/metadata.rs` and the `Field` protocol views (`Field::iceberg`,
   `Field::parquet_field_id`, `Field::mysql`, `Field::spark`). Doris state is
   inert `doris:*` string properties reached through a `Field::doris` view.
7. `rust/src/json/` and `rust/src/text/` — the Stream Load response is JSON and
   is decoded through the shared `Value`, never `serde_json` directly, never a
   hand-rolled scan.
8. `scripts/check_iceberg_interop.py` and `rust/tests/iceberg_interop.rs` — the
   exact interop harness pattern to copy, including the `SKIPPED` word the
   driver fails on so a skipped half can never read as a pass.
9. `docs/iceberg.md`, `docs/parquet.md`, `docs/io.md`, `docs/benchmarks.md` —
   the documentation register to match.

---

## 1. Architecture

### 1.1 Feature and module layout

New non-default feature in `rust/Cargo.toml`:

```toml
# Apache Doris 4.1 interoperability. Not default: it is an engine target on
# top of the record encodings, and a schema-only consumer never reaches it.
doris = ["arrow", "parquet"]
```

The Iceberg bridge inside it is `#[cfg(all(feature = "doris", feature =
"iceberg"))]`, so `--features doris` alone still compiles and still ships the
Parquet half. `--features "parquet iceberg doris"` is the full build the
extensions compile.

New module `rust/src/doris/`, categorized the way `iceberg/` is - modules own
real implementation, never empty shells around a monolith (`AGENTS.md:17`):

| file | owns |
| --- | --- |
| `mod.rs` | the `Doris` namespace value, shared state, re-exports, the module's one-paragraph statement of what is and is not in scope |
| `types.rs` | `DorisType`: the closed Doris 4.1 type enum, its grammar, its `Display`, and the two-way mapping against `DataType` |
| `schema.rs` | `Field` ↔ Doris table schema: the key model, column order, comments, nullability, defaults, and the `doris:*` field properties |
| `variant.rs` | `VARIANT` and `JSON`: schema templates, subcolumn projection, the `DataType` a variant path resolves to |
| `ddl.rs` | `CREATE TABLE`, `CREATE CATALOG`, `DESCRIBE`/`SHOW CREATE TABLE` - rendered and parsed |
| `sql.rs` | `Expression` → Doris SQL text: quoting, literal rendering, precedence, and the refusal list |
| `tvf.rs` | `S3()` / `HDFS()` / `FILE()` / `HTTP()` table-value-function text from a `Url`, a `RecordOptions`, and a predicate |
| `load.rs` | Stream Load: the header set, the body encoded through the three record methods, and `LoadReport` decoded from the JSON response |
| `export.rs` | reading what `EXPORT`, `SELECT INTO OUTFILE`, and `INSERT INTO ... SELECT FROM tvf()` left on storage |
| `catalog.rs` | the Doris external-catalog bridge: Iceberg and Hive catalog text, and the type-mapping check |
| `options.rs` | `DorisOptions`: every knob, resolved explicit → table property → default |
| `tests.rs` | the module's edge cases |

`Doris`, `DorisType`, `DorisOptions`, `StreamLoad`, `LoadReport` are re-exported
from `rust/src/lib.rs` behind the feature, beside the Iceberg exports.

### 1.2 What this module is, and what it is emphatically not

Say this in the module docs, in one short paragraph, so the next reader does
not re-open it:

**In scope.** Everything that is a *value*: the type system, the schema, the
DDL text, the predicate text, the wire *body*, the wire *headers*, the response
*document*, and the on-storage layout Doris reads and writes.

**Out of scope, named not emulated** (`AGENTS.md:228` sets this precedent for
the REST catalog and non-`main` branch writes):

- **No HTTP client.** `StreamLoad` produces a method, a URL, a header map, and
  a body handle. It never opens a socket. An HTTP `IOBase` backend is a sibling
  module and future work; when it exists, `StreamLoad` gains a one-line
  `send` that goes through it. Say that sentence in the docs.
- **No MySQL wire protocol.** Doris's query port is MySQL; that is a network
  client and a second wire format. DDL, catalog, and TVF statements are
  produced as *text* the caller executes with whatever driver it already has.
- **No Arrow Flight SQL client.** Doris 2.1+ serves query results over Flight
  SQL, and it is the fastest way to read from Doris - but it is gRPC, it is
  async, and `IOBase` is neither. Record the measurement it would win
  (published figures put it 20×-100× over the MySQL protocol) and name it as
  future work behind a Flight backend. Do not emulate it, and do not claim a
  number this workspace did not measure.
- **No BE-internal formats.** Segment V3, the tablet layout, and Doris's
  internal indexes are engine internals; the exchange surface is Parquet, ORC,
  CSV, JSON, Arrow IPC, and Iceberg.

### 1.3 `DorisType`: one closed enum, complete for 4.1

`DorisType` is a `#[non_exhaustive]` enum covering **every** type Doris 4.1
spells, grouped and documented as groups, with `Display` canonical and
round-tripping through `FromStr` (`AGENTS.md:604`):

- **Boolean and integers** — `Boolean`, `TinyInt`, `SmallInt`, `Int`,
  `BigInt`, `LargeInt` (16 bytes, signed, range ±2^127).
- **Floating and fixed point** — `Float`, `Double`, `Decimal { precision,
  scale }`.
- **Temporal** — `Date`, `DateTime { precision }` (0..=6, microsecond
  ceiling), `Time { precision }` (query-only: Doris will not store it - the
  mapping must refuse a stored column of it, naming the reason), and
  **`TimestampTz`**, new in 4.1: stored as UTC, converted on read.
- **String and binary** — `Char { length }` (1..=255 bytes),
  `Varchar { length }` (1..=65533 bytes), `String` (default 1 MiB, configurable
  to 2 GiB), `VarBinary` (4.0+, catalog-mapped only - a native Doris table
  cannot declare it, and the mapping says so).
- **Nested, fixed schema** — `Array(Box<DorisType>)`,
  `Map(Box<DorisType>, Box<DorisType>)`,
  `Struct(Arc<[(SmolStr, DorisType)]>)`. Recursive at every level and subject
  to `DataType::PARSE_RECURSION_LIMIT`.
- **Semi-structured** — `Json`, `Variant(Option<VariantTemplate>)` where the
  template is 4.x's `VARIANT<'id': INT, 'tags*': ARRAY<TEXT>>` schema-template
  syntax, wildcards included.
- **Aggregation state** — `Bitmap`, `Hll`, `QuantileState`,
  `AggState(Box<DorisType>)`. These have no logical Arrow shape; they map to
  opaque `Binary` with a `doris:agg-state` property recording the spelling, and
  the docs say plainly that a round trip through Yggdryl preserves the bytes
  and not the semantics.
- **Network** — `Ipv4`, `Ipv6`.

The grammar in `types.rs` follows the parser contract (`AGENTS.md:626`)
exactly: type keywords ASCII case-insensitive, names and quoted values keep
case and Unicode, split only at top-level separators honoring quoting and
escapes, reject trailing tokens and malformed numbers, enforce the recursion
limit, every error carries a byte position and context. It never re-implements
Arrow type parsing - where a Doris type is spelled with an Arrow-compatible
inner type, the text goes to `DataType::from_str`.

### 1.4 The mapping is total, two-way, and honest about what it loses

`types.rs` owns two functions and one table, and nothing else decides a type:

```rust
impl DorisType {
    pub fn to_data_type(&self) -> Result<DataType>;
    pub fn from_data_type(data_type: &DataType) -> Result<Self>;
}
```

The mapping table is written **into the module docs and into
`docs/doris.md` as one table**, generated from the same constant the code uses
so the two cannot drift. Every row states the direction it is lossless in.
The rows that are *not* symmetric are the interesting ones, and each gets a
sentence:

| Doris | `DataType` | note |
| --- | --- | --- |
| `LARGEINT` | `Decimal128(38, 0)` | Arrow has no 128-bit integer; the decimal carries the value exactly up to 38 digits and refuses beyond. `i128::MIN` does **not** fit - reject it by name, do not wrap. |
| `DATETIME(p)` | `Timestamp(unit, None)` | `p` 0..=6 maps to Second/Milli/Micro; Doris has no nanosecond, so a nanosecond timestamp is refused unless `safe` truncation is asked for, and then it is reported. |
| `TIMESTAMPTZ` | `Timestamp(Micro, Some("UTC"))` | Doris stores UTC and converts on read; the timezone is the session's, so a non-UTC Arrow timezone is normalized and the original recorded in `doris:timezone`. |
| `TIME(p)` | `Time64(Micro)` | read-only: Doris 4.1 will not store a `TIME` column. Refuse it on the write path, naming the version. |
| `CHAR(M)` / `VARCHAR(M)` | `Utf8` | `M` is in **bytes**, not characters. A declared length that a UTF-8 payload can exceed is a real failure mode: carry `M` in `doris:length` and validate on the write path. |
| `STRING` | `Utf8` / `LargeUtf8` | the 1 MiB default is a Doris config, not a format limit; name the config key. |
| `VARIANT` | `Utf8` (JSON text) or the template's `Struct` | with a template, the projection is exact and typed; without one, Doris infers - integers become `BIGINT`, decimals become `DOUBLE`, a path with mixed types is promoted to `JSONB`. Say that; do not pretend inference is stable. |
| `BITMAP`/`HLL`/`QUANTILE_STATE`/`AGG_STATE` | `Binary` | bytes preserved, semantics not. |
| `IPV4` / `IPV6` | `FixedSizeBinary(4)` / `FixedSizeBinary(16)` | with `doris:ip` recording which, so the reverse mapping is exact. |
| `ARRAY<T>` | `List(T)` | Doris arrays are always nullable-element; a non-nullable Arrow list element is widened and the widening is reported. |
| `MAP<K,V>` | `Map(K, V)` | Doris map keys are non-null scalars only; a nested or nullable key is refused naming both. |
| `STRUCT<...>` | `Struct(...)` | names are case-insensitive on both sides; an ambiguous fold is refused, never silently picked. |

Arrow types Doris has no home for - `Interval`, `Duration`, `Union`,
`RunEndEncoded`, `Float16`, `Dictionary` of a non-string value, `Decimal256`
beyond 38 digits - are each refused **by name** with `expected X, got Y`
(`AGENTS.md:578`), except where the compatibility walker can widen them
losslessly (see 1.5).

### 1.5 Doris as the sixth compatibility target

Add `Scheme::DORIS` to `COMPATIBILITY_TARGETS` (`enums/scheme.rs:91`) and a
Doris row to the per-target scalar matrix in
`rust/src/datatype/compatibility.rs`. `schema.to_scheme_compat(&Scheme::DORIS)`
returns the schema Doris can actually store, widening exactly what widens
losslessly and refusing the rest:

- `Float16 → Float32`, `Dictionary(_, Utf8) → Utf8`, `RunEndEncoded(_, T) → T`,
  `LargeList → List`, `Utf8View → Utf8`, `Decimal256(p,s)` with `p ≤ 38` →
  `Decimal128(p,s)`;
- `Timestamp(Nano, _) → Timestamp(Micro, _)` **only** when the caller asked for
  it; silently dropping precision is a correctness bug, not a widening;
- `Union`, `Interval`, `Duration`, and `Decimal256` beyond 38 digits are
  refused, naming both sides.

Never fork the walker (`AGENTS.md:707`). Rewrites preserve name, nullability,
and metadata, and invalidate a populated Arrow cache exactly once.

`Field::doris` joins the existing protocol views as the one way to reach
`doris:*` properties - `doris:key-type`, `doris:key`, `doris:aggregate`,
`doris:distribution`, `doris:buckets`, `doris:length`, `doris:ip`,
`doris:agg-state`, `doris:variant-template`, `doris:auto-partition`,
`doris:default`, `doris:comment`. It is a *view* over the one shared snapshot,
not a second map (`AGENTS.md:17`).

### 1.6 `schema.rs` and `ddl.rs`: a `Field` is the table

A struct root `Field` is the schema (`AGENTS.md:17`); `ddl.rs` renders it and
parses it back.

```rust
let sql = doris::create_table(&schema, &DorisOptions::default())?;
let back = doris::schema_from_create_table(&sql)?;
assert_eq!(back, schema);
```

- **Key models**: `DUPLICATE KEY`, `UNIQUE KEY` (merge-on-write), and
  `AGGREGATE KEY` with the per-column aggregation function. The model comes
  from `doris:key-type` on the root and `doris:key` / `doris:aggregate` on the
  columns; absent, it is `DUPLICATE KEY` over the leading columns Doris
  requires, and the docs say which.
- **Distribution**: `DISTRIBUTED BY HASH(...) BUCKETS n` or
  `... BUCKETS AUTO` or `RANDOM`, from `doris:distribution` / `doris:buckets`.
- **Partitioning**: `PARTITION BY RANGE(...)` / `LIST(...)` and 4.x
  `AUTO PARTITION BY RANGE (date_trunc(col, 'day'))`. The partition columns are
  the schema's partition-marked fields - the same marker Iceberg and the Hive
  folder layout already use (`AGENTS.md:109`). One authority on partition
  columns, not a Doris-specific one.
- **Properties**: `replication_num`, `storage_medium`,
  `enable_unique_key_merge_on_write`, `light_schema_change`,
  `variant_max_subcolumns_count`, `variant_enable_flatten_nested`, and the
  4.1 `store_row_column` / DOC-mode keys - each resolved through
  `DorisOptions`, never interpolated ad hoc.
- **Parsing back** is the same recursive grammar discipline: `SHOW CREATE
  TABLE` output and `DESCRIBE` output both round-trip to the same `Field`, and
  every branch gets a round-trip test and an adversarial test
  (`AGENTS.md:626`).

### 1.7 `sql.rs`: the one predicate, rendered for Doris

`Expression` is already the workspace's single filter representation. `sql.rs`
renders a `Bound` predicate as Doris SQL, and renders **nothing else**:

```rust
let predicate: Expression = "ccy = 'EUR' and price > 100 and ts >= timestamp '2026-01-01'".parse()?;
assert_eq!(doris::sql::predicate(&predicate)?, "`ccy` = 'EUR' AND `price` > 100 AND `ts` >= '2026-01-01 00:00:00'");
```

- Identifiers are backtick-quoted with backticks doubled; string literals are
  single-quoted with Doris's escape rules; decimals never become floats;
  temporals render in the exact literal form Doris parses.
- Nodes Doris cannot express (a `&holder.*` selector, a function Doris does not
  have) are **not** rendered - they come back as the *residual* through the
  existing `pushdown.rs` split, exactly as Iceberg's residual already works.
  A caller gets `(pushed_sql, residual_expression)`, applies the first in
  Doris and the second on the batches. Generalize the existing split; do not
  fork it.
- The refusal list is documented: what Doris will be asked, what stays behind,
  and why.

### 1.8 `tvf.rs` and `export.rs`: the round trip that validates the encodings

This is the pair the whole task exists for.

**Out**: Yggdryl writes Parquet (or Iceberg) through the three record methods,
then `tvf.rs` renders the exact statement that makes Doris read it:

```rust
let statement = doris::tvf::select(&url, &options)
    .project(&["ccy", "price"])
    .filter(&predicate)
    .to_string();
// SELECT `ccy`, `price` FROM S3('uri' = 's3://bucket/trades/*.parquet', 'format' = 'parquet', ...)
//  WHERE `ccy` = 'EUR' AND `price` > 100
```

The TVF kind comes from the `Url`'s scheme (`s3`/`oss`/`cos` → `S3()`,
`hdfs` → `HDFS()`, `file` → `LOCAL()`/`FILE()`, `http`/`https` → `HTTP()`,
4.0.2+), the `format` property from the media type via
`RecordOptions::for_media_type` (`AGENTS.md:109` - the encoding is never
guessed and never an argument), and the credential properties from inert
`s3:*` / `hdfs:*` protocol metadata already on the `Field` or `Url`. Path
wildcards (`file_*`, `file_{1..3}`, `file_{a,b}`) come from
`Url::is_glob`/`glob_parts` - never a second glob spelling.

**Back**: Doris writes with `EXPORT`, `SELECT INTO OUTFILE`, or 4.1's
`INSERT INTO FILE()/S3()`; `export.rs` reads that folder back. It is a
`Holder` over a folder and nothing more (`AGENTS.md:109` - a handle plus
`RecordOptions` is the whole surface, no dataset type): the leaves' media type
selects the encoding, the folder's `column=value` layout restores partition
columns through `io/partition.rs`, and the caller gets a `BatchReader`. Doris's
own naming convention (the `<label>_<n>.parquet` suffix, the optional success
marker) is recognized, and anything unrecognized is *reported*, never skipped
silently.

**Symmetry is the assertion**: rows written → rows Doris reads → rows Doris
exports → rows read back must be identical, value for value, including nulls,
decimals to the scale, timestamps to the unit, and every nested level.

### 1.9 `load.rs`: Stream Load, composed not sent

```rust
let load = doris::StreamLoad::new("trades", "orders")
    .with_format(MimeType::PARQUET)
    .with_label("2026-08-19-batch-7")
    .with_merge(MergeType::Merge)
    .with_options(&options);
let (headers, body) = load.compose(reader)?;   // body is an IOBase handle
```

- The URL shape is `PUT /api/{db}/{table}/_stream_load`, with the FE→BE
  307 redirect documented (the FE picks a BE round-robin as coordinator).
- The header set is a typed value, not a string map the caller fills: `label`,
  `format` (`csv`, `csv_with_names`, `csv_with_names_and_types`, `json`,
  `parquet`, `orc`, `arrow`), `column_separator`, `line_delimiter`, `columns`,
  `jsonpaths`, `json_root`, `strip_outer_array`, `where`, `partitions`,
  `max_filter_ratio`, `strict_mode`, `timezone`, `timeout`, `merge_type`,
  `delete`, `compress_type` (**including 4.1.3's zstd**), `enclose`, `escape`,
  `skip_lines`, `trim_double_quotes`, `hidden_columns`,
  `function_column.sequence_col`, `unique_key_update_mode`,
  `partial_update_new_key_behavior`, `two_phase_commit`, `group_commit`, and
  4.x's `compute_group`. Each is one field, each resolved through
  `DorisOptions` (explicit → property → default), each with a typed error
  naming key and value when unparseable - never a silent default
  (`AGENTS.md:228` states this rule for Iceberg's size key; it holds here).
- The **body is produced through the three record methods**, into a `Buffer`
  or any other `IOBase` handle. `format: arrow` writes an Arrow IPC stream
  through `rust/src/ipc/`; `format: parquet` through `rust/src/parquet/`;
  `format: json`/`csv` through the shared text tier. There is no fourth
  writer, and nothing is collected that could stream: the body handle is
  written by a `BatchReader` and read back by the caller in chunks, so a load
  larger than memory is expressible (`AGENTS.md:109`).
- `LoadReport::from_json` decodes the response document through
  `rust/src/json/` into typed fields - `TxnId`, `Label`, `Status`
  (`Success` | `Publish Timeout` | `Label Already Exists` | `Fail`),
  `ExistingJobStatus`, `Message`, `NumberTotalRows`, `NumberLoadedRows`,
  `NumberFilteredRows`, `NumberUnselectedRows`, `LoadBytes`, `LoadTimeMs`,
  and the per-phase timings, plus `ErrorURL`. A non-`Success` status is a typed
  error carrying the message and the error URL, so a caller can fix the input
  without reading source.

### 1.10 `catalog.rs`: the Iceberg bridge

Behind `#[cfg(all(feature = "doris", feature = "iceberg"))]`:

- `doris::catalog::create_catalog(&spec)` renders the `CREATE CATALOG`
  statement for the catalog types Doris 4.1 supports - `hms`, `rest`,
  `hadoop`, `glue`, `dlf`, `s3tables`, and 4.1.0's experimental `jdbc`
  (PostgreSQL/MySQL/SQLite) - from the same inert protocol metadata the
  `iceberg::Catalog` already carries. A warehouse folder Yggdryl created is
  addressable as a `hadoop` catalog with no further configuration; say that,
  and show it.
- `doris::catalog::check_readable(&table)` walks a committed
  `iceberg::Table`'s schema through `DorisType::from_data_type` and reports,
  per column, whether Doris 4.1 can read it - so a table is known unreadable
  *before* an interop run says so. It never mutates the table.
- The support matrix is documented as fact, not aspiration: Doris 4.1 reads
  and writes Iceberg **V1 and V2** fully (`INSERT INTO`, `INSERT OVERWRITE`,
  `UPDATE`, `DELETE`, `MERGE INTO`, `CTAS`), and reads **V3** including
  Puffin-format deletion vectors, with V3 write support arriving through the
  same. Position deletes and equality deletes are both read. Time travel is
  `FOR TIME AS OF` / `FOR VERSION AS OF`, and branches and tags are
  `table@branch(name)` / `table@tag(name)`. Yggdryl writes V2; the interop
  therefore exercises V2 in both directions and V3 read-only, and the docs say
  exactly that rather than implying more.

---

## 2. Order of work (`AGENTS.md:9` — Rust first, fully)

**Phase 0** research note → **Phase 1** Rust core complete (types, mapping,
compat target, schema, DDL, variant, SQL, TVF, load, export, catalog, options,
tests, interop, benches, docs) → **Phase 2** optimization pass → **Phase 3**
Python → **Phase 4** JavaScript → **Phase 5** docs and benchmark tables →
**Phase 6** required checks. Phase 1 stopping on its own is complete work.

---

## 3. Phase 0: pin the target, from the sources

Before writing the enum, spend one pass on the primary sources and write
`docs/doris.md`'s design section from it - short, cited, opinionated, not a
survey. **Verify every number below against the live docs; they are what this
prompt was written from, in August 2026, and they are what you must confirm or
correct:**

- **The version.** Target **Apache Doris 4.1**, latest patch **4.1.3**
  (2026-07-13; 4.1.0 was 2026-04-21, 4.1.1 2026-05-24, 4.1.2 2026-06-17). The
  4.0 line is still receiving patches (4.0.8, 2026-08-14) - support 4.0 as the
  floor and gate 4.1-only spellings (`TIMESTAMPTZ`, `MERGE INTO`, `UNNEST`,
  variant DOC mode, `INSERT INTO FILE()`, zstd stream-load compression) behind
  a declared version so a 4.0 cluster gets a typed refusal instead of a syntax
  error from the server.
  <https://doris.apache.org/releases/core/>
- **What 4.1 changed that this module must know**: `TIMESTAMPTZ`; Segment V3
  (external column metadata separation, sparse-column sharding, DOC mode for
  deferred JSON materialization); full Iceberg V2/V3 with `MERGE INTO`;
  Iceberg sorted write and manifest cache; a Parquet page cache reported at
  >20% on scans; `UNNEST`; recursive CTE; `ASOF JOIN`; JDBC Iceberg catalog;
  vector indexing (out of scope, but do not let the enum forget the column
  types it introduces).
  <https://doris.apache.org/releases/v4.1/release-4.1.0/>
- **The type system**, every type and every parameter:
  <https://doris.apache.org/docs/4.x/sql-manual/basic-element/sql-data-types/data-type-overview>
- **VARIANT**, including schema templates, `variant_max_subcolumns_count`
  (default 2048, practical ceiling ~10 000), sparse-column sharding, the
  BIGINT/DOUBLE/JSONB inference rules, and DOC mode:
  <https://doris.apache.org/docs/4.x/sql-manual/basic-element/sql-data-types/semi-structured/VARIANT>
- **Stream Load**, every header and every response field:
  <https://doris.apache.org/docs/4.x/data-operate/import/import-way/stream-load-manual>
- **EXPORT / SELECT INTO OUTFILE**, formats and compression (Parquet: SNAPPY
  default, plus GZIP, BROTLI, ZSTD, LZ4, PLAIN; ORC: ZLIB default, plus PLAIN,
  SNAPPY, ZSTD; `max_file_size` 5 MiB..2 GiB, default 1 GiB):
  <https://doris.apache.org/docs/4.x/sql-manual/sql-statements/data-modification/load-and-export/EXPORT>
- **Table value functions**, syntax, properties, wildcards, and 4.1.0's
  `INSERT INTO tvf()` export:
  <https://doris.apache.org/docs/4.x/lakehouse/file-analysis>
- **The Iceberg catalog**, catalog types and the operation matrix:
  <https://doris.apache.org/docs/4.x/lakehouse/catalogs/iceberg-catalog>
- **Arrow Flight SQL**, for the future-work note and nothing else:
  <https://doris.apache.org/docs/4.x/db-connect/arrow-flight-sql-connect>

Read the **Doris source** where the docs are ambiguous - `apache/doris` on
GitHub - specifically the Parquet reader (`be/src/vec/exec/format/parquet/`)
and the Iceberg reader, because the questions this task actually answers are
"which Parquet logical types does Doris's own C++ reader accept" and "which
Iceberg V2 delete shapes does it apply". A doc sentence is weaker evidence
than the reader.

**The deliverable of this phase is a decision list in the docs**: which Doris
types map losslessly, which map lossily and how, which are refused, which
spellings are 4.1-only, and which surfaces (Flight SQL, MySQL wire, Segment
V3) are deliberately out of scope with the reason.

---

## 4. Phase 1 details: tests

`rust/src/doris/tests.rs` plus per-file test modules where the existing modules
keep them, and the interop target below. Cover, at minimum:

### 4.1 Every type, exhaustively

A single table-driven test walks **every** `DorisType` variant and asserts, for
each: `Display` round-trips through `FromStr`; `to_data_type` then
`from_data_type` returns the original or a documented widening; the error
message for a refused mapping names both sides. A `#[test]` that iterates a
`const ALL: [DorisType; N]` and fails when a new variant is added without a
case - so the exhaustiveness is enforced by the compiler and the test, not by
review.

Parameters are boundary-tested, not sampled: `CHAR(1)`, `CHAR(255)`,
`CHAR(256)` refused; `VARCHAR(1)`, `VARCHAR(65533)`, `VARCHAR(65534)` refused;
`DECIMAL(1,0)`, `DECIMAL(38,38)`, `DECIMAL(39,0)` refused; `DATETIME(0)`,
`DATETIME(6)`, `DATETIME(7)` refused; `LARGEINT` at ±(2^127−1) and the refusal
at `i128::MIN`.

### 4.2 Deep nesting

The nesting tests are not decoration - they are where a mapping fails in
production. Build and round-trip, in **both** directions and through **both**
Parquet and Iceberg:

- `ARRAY<ARRAY<ARRAY<INT>>>` — three levels;
- `MAP<STRING, ARRAY<STRUCT<a: INT, b: MAP<STRING, DECIMAL(18,4)>>>>` — the
  four-way alternation that breaks naive mappers;
- `STRUCT` nested to the recursion limit, and one past it (the error carries
  the byte position and the path);
- a struct whose children are every scalar type, inside a list, inside a map
  value — so one fixture exercises the whole scalar matrix at depth;
- nulls at **every** level: a null map, a map with a null value, a list with
  null elements, a struct with all-null children, a non-null struct containing
  a null list containing non-null structs. Wrapper exposure must not make
  hidden child nulls observable (`AGENTS.md:707`).
- `VARIANT` with and without a template, including a path whose type changes
  between rows (the JSONB promotion), a path count over
  `variant_max_subcolumns_count`, and a nested object inside an array (which
  Doris flattens differently - assert what it actually does, do not assume).

### 4.3 DDL and SQL

Round trip `Field` → `CREATE TABLE` → `Field` for every key model, every
distribution, range/list/auto partitioning, and every property. Parse real
`SHOW CREATE TABLE` and `DESCRIBE` output captured from a live 4.1 into
fixtures. Adversarial: unbalanced backticks, a comment containing a backtick, a
default value containing a quote, a duplicate column name, a partition column
not in the schema, trailing tokens. Every error carries a byte position.

Predicate rendering: an assertion table of `Expression` → Doris SQL for every
node kind, plus the residual split for every node Doris cannot take.

### 4.4 Stream Load and export

Header composition for every format and every option combination, with a
golden-file assertion. Body bytes for `parquet`, `arrow`, `json`, and `csv`
from the same `BatchReader`, each decoded back through the matching Yggdryl
reader and compared row for row. `LoadReport` decoding for every `Status`,
including a malformed document and a document missing a field. A body larger
than the batch size streams: assert peak retained batches is one, do not claim
it (`AGENTS.md:707`).

Export reading: a folder of Doris-named Parquet files, a folder of ORC, a
folder of CSV with and without the header variants, a Hive-partitioned export
whose partition columns must come back typed, an empty export, and a folder
containing one unrecognized file (reported, not skipped).

### 4.5 Interop, both directions — `rust/tests/doris_interop.rs`

Copy the Iceberg harness pattern exactly (`AGENTS.md:228` - exchange formats
are validated against an outside implementation):

- `scripts/check_doris_interop.py` is the driver. It brings up Apache Doris
  **4.1.3** in Docker (FE + BE, the official `apache/doris` images), waits for
  readiness, and runs both halves.
- **Half one, Yggdryl → Doris.** The cargo target writes, into
  `target/doris-interop/from-rust`: a Parquet file with every scalar type; a
  Parquet file with the deep-nested fixtures from 4.2; a partitioned Parquet
  folder; and an Iceberg V2 table with an append and a key-matched merge. The
  driver then makes Doris read each one - the Parquet through
  `SELECT ... FROM LOCAL()/S3()`, the Iceberg through a `hadoop` catalog it
  creates from the warehouse folder - and compares **every row and every
  nested value** against what was written, not just counts.
- **Half two, Doris → Yggdryl.** Doris writes the same rows out with
  `INSERT INTO FILE()` (Parquet, ORC, CSV) and commits an Iceberg V2 table with
  `INSERT INTO`, an `UPDATE`, a `DELETE`, and a `MERGE INTO` - so the table
  carries position deletes and equality deletes. The cargo target reads all of
  it back and asserts the rows.
- **Half three, the wire.** The driver sends a real Stream Load with the body
  and headers `load.rs` composed, for `parquet`, `arrow`, `json`, and `csv`,
  and asserts the returned `LoadReport` matches what `LoadReport::from_json`
  decodes and that `NumberLoadedRows` equals the rows written.
- Run alone, the Rust target prints **`SKIPPED`** when the external artifacts
  are absent, and the driver fails on that word - so a skipped half can never
  read as a pass.
- A CI job runs the driver; it is not part of `cargo test`, because it needs a
  server. `docs/testing.md` says how to run it locally.

---

## 5. Phase 1 benchmarks — where the protocols get validated

`rust/benchmarks/doris.rs` with the dispatcher pattern
(`#[path = "doris/mod.rs"] mod benchmarks;`, stable Criterion group IDs), plus
the interop-driven measurements in `scripts/check_doris_interop.py`. The point
of this benchmark is **not** to show Yggdryl is fast at rendering SQL. It is to
put a number on the read/write protocols against an engine that did not come
from this workspace.

**In-process Criterion groups** (no server):

- `doris_types` — mapping and grammar: `DorisType::from_str` on a scalar, on
  the four-way nested type, on a variant template; `to_data_type` /
  `from_data_type` both directions. Baseline: the existing
  `DataType::from_str` groups, so the reader sees this grammar costs the same
  order as the schema grammar.
- `doris_ddl` — `CREATE TABLE` render and parse against a 10-column, a
  200-column, and a deeply-nested schema.
- `doris_sql` — predicate rendering and the residual split, against the
  `expression_bind` groups.
- `doris_load` — body composition per format for a 1e6-row batch, reported as
  **throughput in rows and in bytes-on-the-wire**, so the four formats are
  directly comparable. This is the number that decides which format a caller
  should use, and it is the first thing the docs table shows.
- `doris_export` — reading a Doris-shaped export folder, against reading the
  same bytes as a plain Parquet folder, so the layout handling's cost is
  visible as a delta and not a total.

**Server-driven measurements** (the driver, release build, numbers into
`docs/benchmarks.md`, regenerated never edited, naming machine, Doris version,
and build profile):

1. **Write protocol.** Yggdryl writes N rows to Parquet; Doris reads them via
   TVF. Report Yggdryl's write time, the file bytes, and Doris's scan time.
   Baseline: the **same rows written by PyArrow**, read by the same Doris query.
   If Doris scans Yggdryl's Parquet slower than PyArrow's, that is a real
   finding about row-group sizing or encodings - chase it, do not report it and
   move on.
2. **Read protocol.** Doris exports N rows to Parquet and ORC; Yggdryl reads
   them. Baseline: PyArrow reading the identical files. Same rule.
3. **Iceberg both directions.** Yggdryl commits a partitioned V2 table; Doris
   plans and scans it - report files read vs skipped, which the plan already
   reports as a testable number (`AGENTS.md:228`). Then Doris commits with
   `MERGE INTO`; Yggdryl plans and scans that. Baseline: PyIceberg on both
   tables, which the workspace already runs.
4. **Stream Load formats.** The same 1e6 rows loaded four ways -
   `parquet`, `arrow`, `json`, `csv` - reporting bytes on the wire, server-side
   `LoadTimeMs`, `ReadDataTimeMs`, and `WriteDataTimeMs` from the real
   `LoadReport`. Plus zstd `compress_type` (4.1.3) against uncompressed.
5. **Pushdown.** The same query with and without the projection and predicate
   the TVF renderer emits, reporting bytes Doris actually read. Pushdown that
   does not reduce bytes read is pushdown that does not work.

Python and JavaScript benchmarks compare the boundary crossing against
implementations the reader trusts on the same payload: `pyarrow.parquet` and
`pyarrow.dataset`, and - if available in the bench environment without becoming
a package dependency - the `mysql-connector-python` and ADBC paths a Doris user
would otherwise take.

---

## 6. Phase 2: the optimization pass (find them, measure them, then land them)

This is a distinct phase with a distinct rule from `AGENTS.md:707`: **measure
before claiming any optimization**, and an optimization that changes observable
behavior is a bug. Work the list below; for each item, land it with a
benchmark delta, or record in `docs/doris.md` that it was tried and did not pay
- a refused optimization with a number beside it is a real deliverable.

1. **Parquet writer settings Doris's reader actually likes.** Sweep row-group
   size, page size, dictionary encoding, compression (SNAPPY / ZSTD / LZ4), and
   statistics granularity, measured by *Doris's* scan time, not ours. Doris
   4.1 added a Parquet page cache reported at >20%; find the page size that
   cooperates with it. Land the winning defaults in `DorisOptions` with the
   measurement in the commit message.
2. **Format choice for Stream Load.** Rank `arrow`, `parquet`, `json`, `csv` by
   bytes-on-the-wire and by server-side load time. Arrow IPC should win on CPU
   (no serialization on either side) and lose on bytes; prove which matters at
   which row count and document the crossover.
3. **Streaming body with a bounded buffer.** The composed body must never
   require holding the whole load. Use chunked transfer encoding, hold one
   batch, and assert the peak.
4. **zstd stream-load compression** (4.1.3): measure the bytes/CPU trade
   against uncompressed on the same payload.
5. **Projection and predicate pushdown into the TVF text**, measured as bytes
   Doris read - see benchmark 5 above.
6. **Iceberg write alignment.** Doris 4.1 added Iceberg sorted write and a
   manifest cache. Check whether Yggdryl's `write.target-file-size-bytes`
   default and its manifest granularity cooperate with Doris's planner; a
   manifest layout that defeats Doris's cache is a real cost with a real
   number.
7. **Allocation on the hot paths.** DDL rendering, predicate rendering, and
   header composition must not allocate per column or per node beyond the
   single output buffer; core scalar construction, getters, and iteration setup
   must not allocate at all (`AGENTS.md:707`). Prove it with an allocation
   baseline the way the codec benchmarks already do.
8. **One mapping table, computed once.** The Doris↔Arrow matrix is a `const`
   consulted by index, never a match chain re-walked per column, and never a
   per-record map.

Every landed optimization is invisible to a caller and stated in a comment; a
silent cap - a top-N, a sampling, a truncation - is worse than none.

---

## 7. Phase 3: Python binding

`python/src/doris.rs` exposing a `yggdryl.doris` namespace over the native
module - no Python-side type mapping, no Python-side SQL rendering
(`AGENTS.md:841`):

- `doris.DorisType.parse("map<string, array<struct<a:int>>>")`,
  `.to_data_type()`, `DorisType.from_data_type(dt)`, `str(...)` canonical.
- `doris.create_table(schema, **options) -> str`,
  `doris.schema_from_create_table(sql) -> Field`.
- `doris.tvf(url, **options).project([...]).filter("price > 100") -> str`,
  accepting `str | Expression` for the filter exactly as the existing filter
  arguments do.
- `doris.StreamLoad(db, table, format=..., label=...)` with
  `.compose(reader) -> (dict[str, str], IOBase)`, and
  `doris.LoadReport.from_json(...)`. The docstring says, in one line, that
  nothing is sent.
- `doris.check_readable(table)` for the Iceberg bridge.
- `schema.to_scheme_compat("doris")` works through the existing method with no
  new spelling; `Field.doris` joins the protocol views.
- `_native.pyi` and `__init__.pyi` updated; `mypy --strict` green.
- `python/tests/test_doris.py` in house style: the full type matrix, the deep
  nesting fixtures, DDL round trips, and - as the outside check - a live-Doris
  test marked `@pytest.mark.doris` that skips loudly when no server is
  configured, exercising Stream Load and the TVF round trip against the same
  container the interop driver uses.

## 8. Phase 4: JavaScript binding

`node/src/doris.rs`, mirroring the Python surface with camelCase names in a
`doris` loader namespace, the way `iceberg` already is a namespace
(`AGENTS.md:1064`): `DorisType.parse`, `dorisType.toDataType()`,
`doris.createTable(schema, options)`, `doris.schemaFromCreateTable(sql)`,
`doris.tvf(url, options).project([...]).filter('price > 100')`,
`doris.StreamLoad`, `doris.LoadReport.fromJson`. 64-bit values cross as
`bigint`; `LARGEINT` crosses as a decimal string and the docs say why. Errors
surface the native message unchanged. `node/tests/doris.test.js` +
`doris.types.ts` (node:test + `tsc --noEmit`): the type matrix, nesting, DDL
round trips, and type-level checks for the builder.

---

## 9. Phase 5: documentation

- New page `docs/doris.md` — one H1, exactly one opening sentence, then
  example-first sections: map a schema; the full type table (generated, not
  hand-kept); create a table; render a TVF read; compose a Stream Load; read an
  export; the Iceberg bridge; the decision list from Phase 0; the scope note
  from 1.2 saying plainly what is not here and why; the optimization findings
  from Phase 2.
- Every example in **Rust → Python → JavaScript tabs, in that order**, each
  idiomatic, self-contained, with at least one assertion, all passing
  `python scripts/check_docs_examples.py`. Check `.api-bindings.txt` before
  showing a language do anything; a surface the bindings do not expose shows
  Rust alone under `!!! note "Rust only"`.
- Add the page to `mkdocs.yml` beside `iceberg`, and state in the commit why
  that slot. Update `docs/iceberg.md` (the Doris bridge), `docs/parquet.md`
  (Doris as an outside reader), `docs/io.md` (the export layout),
  `docs/testing.md` (how to run the interop driver), and
  `docs/architecture.md`. Regenerate notebooks with
  `python scripts/build_docs_notebooks.py` (edit blocks, never notebooks).
  Update the README layout table for `rust/src/doris/`.
- `docs/benchmarks.md` regenerated from real runs, release builds only, naming
  the Doris version the numbers came from.
- `python -m mkdocs build --strict` stays green.

---

## 10. Phase 6: required checks (all must pass before handoff)

Per `AGENTS.md:1128`: `cargo fmt --check`; warning-free
`cargo clippy --locked --workspace --all-targets -- -D warnings` **twice**
(default features and `--features "parquet iceberg doris"`); workspace tests
twice the same way; `cargo doc` with `RUSTDOCFLAGS="-D warnings"`; the Rust
1.85 core check (default features and `--no-default-features --lib`);
`cargo bench --benches --no-run`; `python scripts/check_doris_interop.py`;
maturin develop + pytest + `mypy --strict`; `npm run test:package` +
`npm test`; `python scripts/check_docs_examples.py`;
`python -m mkdocs build --strict`. Clean generated targets, venvs, Docker
containers and volumes, and `node_modules` after validation.

---

## 11. Hard constraints, restated

- **No new dependency** in any of the three manifests. No HTTP client, no
  MySQL driver, no gRPC or Flight stack, no `serde_json` beyond what is already
  pinned, no Doris crate. The Docker image and the Python `requests` used by
  the interop driver are test-only tooling in `requirements-docs.txt`'s
  sibling, never a runtime dependency.
- **One representation, everywhere.** The filter is `Expression`. The schema is
  a struct `Field`. The encoding comes from the media type. Partition columns
  come from the partition marker. Errors are `crate::Error`. Doris does not get
  a private copy of any of them.
- **Never a second parser, a second cast, or a second error enum.** Arrow type
  text goes to `DataType::from_str`; value conversion goes through
  `field/cast`; the compatibility rewrite goes through the one walker.
- **The record surface stays three methods.** A Stream Load body and an export
  read go through `read_arrow_batch_reader` /
  `write_arrow_batch_reader` / `append_arrow_batch_reader`, never a private
  encoder. Nothing that could stream is collected.
- **Total or refused.** A type mapping either round-trips, or widens with the
  widening documented, or errors naming both sides. There is no third outcome
  and no silent coercion.
- **Measure before claiming.** Every optimization in Phase 2 carries a number
  or a note saying it did not pay.
- Method names follow the exact vocabulary (`AGENTS.md:409`); Rust
  `create_table`/`schema_from_create_table`/`check_readable` ↔ Python the same
  ↔ JS `createTable`/`schemaFromCreateTable`/`checkReadable`; argument names and
  order identical across languages.
- Commit in coherent steps (types, mapping, compat target, schema, DDL,
  variant, SQL, TVF, load, export, catalog, options, interop, benches,
  optimizations, python, node, docs) with descriptive messages; push the
  branch; do not open a PR.

**Definition of done**: a user writes a schema with every Doris 4.1 type in it,
nested four levels deep, in any of the three languages; Yggdryl renders the
`CREATE TABLE`, writes the rows as Parquet and commits them as an Iceberg V2
table, and composes the Stream Load; a real Apache Doris 4.1.3 reads all three
and returns every value unchanged; Doris then writes the same rows back out
through `MERGE INTO` and `INSERT INTO FILE()`, Yggdryl reads them, and the rows
are identical again - and `docs/benchmarks.md` shows what each protocol cost,
beside PyArrow and PyIceberg on the same payload.
