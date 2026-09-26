---
name: yggdryl
description: Start here for any code that uses the yggdryl package - the Rust crate, the Python wheel or the npm package - for Arrow-native schemas (DataType, Field, Scalar), columns (Serie), storage handles (IOBase over local files, ZIP, S3/GCS/Azure), record media (Arrow IPC, Parquet, Avro, text, Iceberg), JSON/YAML/TOML/XML, URIs, expressions, xxHash/TxHash, FIX and market data. Use when installing or importing yggdryl, choosing which yggdryl API answers a task, translating yggdryl code between Rust, Python and Node.js, or before any other yggdryl-* skill, whose shared conventions (naming, defaults, errors, streaming, zero copy) live here.
---

# Yggdryl

One Rust core owns every type, parser, codec and storage backend; the Python
and Node.js packages are native views of the same values and implement
nothing of their own. So a behaviour, an error message and a default are the
same in all three languages - only the spelling changes (`snake_case` in Rust
and Python, `camelCase` in JavaScript).

## Install

| | Rust | Python | Node.js |
| --- | --- | --- | --- |
| package | `yggdryl = "0.1"` in `Cargo.toml` | `pip install yggdryl` | `npm install yggdryl` |
| minimum | Rust 1.85 (1.94 with `iceberg`) | Python 3.10, `pyarrow>=18` | Node 18, `apache-arrow` (a dependency) |
| optional parts | features, all off by default: `parquet`, `iceberg` (implies `parquet`), `aws`, `s3` (implies `aws`) | everything built in | everything built in |
| extras | Arrow is `arrow-*` 59 | the `ygg` CLI ships in the wheel | `yggdryl/replay`, `yggdryl/web/*` |

A Rust build that reads or writes Parquet or Iceberg, or touches an object
store, must enable that feature; the bindings already carry all of them.

## The model

| Noun | Is | Never |
| --- | --- | --- |
| `DataType` | a shape: no name, no nullability, no metadata | a schema |
| `Field` | a `DataType` + name + nullability + `<SCHEME>:<property>` metadata | stored beside a separate datatype |
| schema | a **non-null Struct `Field`**; its children are the columns | a class of its own - there is no `Schema` type |
| `Scalar` | one value; a row is an ordered `Scalar` sequence, named input (`dict`, object) is sorted and canonicalized against the Struct field | a map you index by position |
| `Serie` | a column: the Arrow buffers of one `Field`, or a schema-free run | a list of scalars |
| `ChunkedSerie` | columns of one field kept apart: a chunked array, or a table of one batch per chunk | joined behind your back |
| `SerieReader` | a stream: one record `Serie` per batch, under one cast plan | a `Scalar` |
| `IOBase` | a positional byte handle on any backend (`Holder`), and the record surface (`IOMedia`) over it | a second storage API per backend |
| `Expression` / `Plan` | a grammar over the above: parse once, bind once, evaluate or push down | a second query engine |

The stack a read climbs: **bytes** (`IOBase`: `read_range_bytes`) ->
**records** (`IOMedia`: `read_arrow_reader`) -> **columns** (`Serie`,
`SerieReader`) -> **values** (`Scalar`). Enter at the highest level that
answers the task.

## Choose the skill

| Task | Skill |
| --- | --- |
| declare a schema, parse a type expression, build or check a value, dataclass/record classes, metadata | `yggdryl-types` |
| Arrow arrays/batches/readers, pyarrow/pandas/polars/Arrow JS in or out, columns, casts | `yggdryl-arrow` |
| open a file, bytes, local/ZIP/S3/GCS/Azure, gzip/zlib/zstd, charsets, digests of a handle | `yggdryl-storage` |
| parse or build a URI, URL, URN, ARN, path, glob, hive partition path | `yggdryl-uri` |
| read or write rows/batches in Arrow IPC, Parquet, Avro, text, Iceberg; partitions; merge/upsert | `yggdryl-records` |
| JSON, JSON Lines, YAML, TOML, XML documents to and from values | `yggdryl-documents` |
| filters, selections, SQL-like plans, predicate pushdown, field paths | `yggdryl-expressions` |
| xxHash digests, stable hashes, row digests, TxHash | `yggdryl-hashing` |
| FIX messages, dictionaries, captures, the `ygg fix` CLI | `yggdryl-fix` |
| orders, quotes, executions, order books, market data replay | `yggdryl-market-data` |

## Cross-language conventions

| Concept | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| import | `use yggdryl::{DataType, Field, Scalar};` | `from yggdryl import DataType, Field, Scalar` | `const { DataType, Field, Scalar } = require('yggdryl')` |
| type from text | `DataType::from_str("decimal(18,4)")?` | `DataType("decimal(18,4)")` | `new DataType('decimal(18,4)')` |
| field | `dtype.required_field("id")`, `Field::new(name, dtype, nullable)` | `Field("id", "int64", nullable=False)` | `new Field('id', 'int64', false)` |
| struct / schema | `StructType::from_fields([...])?` | `DataType.from_fields([...])` | `DataType.fromFields([...])` |
| value through a type | `dtype.scalar(v)?`, `field.scalar(v)?` | `dtype.scalar(v)`, `field.scalar(v)` | `dtype.scalar(v)`, `field.scalar(v)` |
| value inferred | `Scalar::from(v)` | `Scalar.from_(v)` | `Scalar.from(v)` |
| value out | match the variant, `as_*` | `.as_py()` | `.asJs()` |
| stable hash | `stable_hash() -> u64` | `.stable_hash() -> int` | `.stableHash() -> bigint` |
| handle | `Holder::from_url(&url, props)?`, `holder::Buffer::new()`, `local::LocalFile` | `IOBase(path_or_url)`, `IOBase.from_bytes()` | `new IOBase(pathOrUrl)`, `IOBase.fromBytes()` |
| whole bytes | `read_all_bytes()`, `write_all_bytes(..)` | `read_bytes()`, `write_bytes(..)` | `readBytes()`, `writeBytes(..)` |
| omitted optional | `Option::None` / builder not called | argument left out (`...` default) | `undefined` |
| clear an optional | a `clear_*`/`remove_*` call | `None` | `null` |
| 64-bit integers | `i64`/`u64` | `int` | `number` when safe, else `bigint`; pass `bigint` in |
| bytes | `&[u8]`, `Vec<u8>` | `bytes` | `Buffer` / `Uint8Array` |
| Arrow | `arrow-array` 59 types, shared buffers | pyarrow over the C Data Interface, **zero copy** | apache-arrow over IPC, **copied** |
| errors | `yggdryl::Error`, `yggdryl::arrow::Error` (`?` converts both ways) | `ValueError` (bad input), `TypeError` (wrong kind), `OSError` subclasses (I/O), each with the native message | `Error` with the native message; arithmetic throws `TypeError`/`RangeError` with `ERR_YGGDRYL_*` codes |

Verb prefixes mean the same everywhere: `from_*` parses or constructs,
`into_*` converts (may allocate), `as_*` borrows without allocating, `is_*` /
`has_*` ask, `get*` looks up, `set_*` validates and mutates in place (failure
leaves the value unchanged), `with_*` returns an updated copy. There is no
`to_*`. Record options are one `RecordOptions` object, and every read or write
also takes its properties by name: `read_arrow_reader(rowheader=...)` in
Python, `readArrowReader({ rowheader })` in JavaScript.

## Rules for fast, correct use

1. **Resolve once at the boundary.** Parse a `DataType`/`Field` once and pass
   the object; compile an `ArrowCastPlan` once per stream; bind an expression
   once. Anything parsed or compiled inside a row or batch loop is a defect.
2. **Stream; never collect.** Record reads answer a batch reader
   (`read_arrow_reader`, `read_arrow`); keep it a reader end to end. Nothing
   streamable should become a list of batches or rows unless the caller asked
   for one.
3. **Values enter through their type.** `DataType.scalar`/`Field.scalar` check
   and canonicalize (width narrowed, decimal rescaled, time at its unit, text
   trimmed). Never shape a value with host-runtime casting (`pyarrow.compute.cast`,
   Arrow JS builders) first - they know none of these rules.
4. **Name the width on the type.** Width, unit, scale and zone belong to the
   `DataType` (`"decimal(18,4)"`, `"timestamp(us, UTC)"`); the value door then
   reads a bare number at that unit.
5. **Storage cost is the call count.** Construction touches nothing, a missing
   resource reads as empty, a write creates the resource and its parents. Act
   once and handle the typed result - never guard with `exists`, `is_dir` or
   `mkdir` first.
6. **The media type picks the encoding.** A suffix (`.parquet`, `.arrows`,
   `.json.gz`) or an explicit `MediaType` decides format and compression; no
   read or write takes a format argument.
7. **Push work down.** Put `select`, `filter`, row bounds and the declared
   `field` in `RecordOptions` so the medium projects, prunes and casts in one
   pass, instead of filtering rows after reading them.
8. **Paths are `FieldPath`s.** `a.b[0]['key']` is parsed once by the grammar;
   never split a path string on `.` yourself.
9. **Errors are the answer.** A refusal names expected, actual and location
   (`$[3].bid.live[0].miccode`, byte position, column). Fix the input it names;
   do not retry or widen the type to make it pass.

## Pitfalls

- Looking for `Schema`, `Record` or `Row` classes: the schema is a non-null
  Struct `Field`; a row is a `Scalar`; a column is a `Serie`.
- Expecting `list<...>` back: the serie family displays as `serie`
  (`DataType("list<int64>")` reads and prints `serie(field("item",int64,...))`).
  The old `list` spellings are accepted on input only.
- Enabling nothing in `Cargo.toml` and then calling Parquet, Iceberg or S3
  APIs: they do not exist without their feature.
- Python: a `str` given to a structured-text loader is document content, not
  a path - pass `pathlib.Path`. Passing `None` to an optional argument clears
  it; leave the argument out to keep the default.
- JavaScript: 64-bit integer columns come back as `bigint` once they pass
  `Number.MAX_SAFE_INTEGER`; write `1n`, not `1`, into `int64` columns when
  building Arrow JS vectors. Arrow JS interop copies through IPC - cross the
  boundary in whole batches, not per row.

## Language references

- `references/rust.md` - features, imports, error types, logging, a runnable end-to-end example.
- `references/python.md` - package layout, typing, pyarrow/pandas/polars, logging, a runnable end-to-end example.
- `references/javascript.md` - CommonJS/TypeScript, `bigint`, `Buffer`, Arrow JS, errors, a runnable end-to-end example.

## Deeper

- Site: https://platob.github.io/yggdryl/ (every example there runs in CI, in all three languages).
- Getting started: https://platob.github.io/yggdryl/getting-started/
- Architecture and shared rules: https://platob.github.io/yggdryl/architecture/
- Rust API: https://docs.rs/yggdryl
