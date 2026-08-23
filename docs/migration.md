# Migration notes

This release intentionally removes the legacy record/decorator vocabulary. There are no
compatibility aliases: update call sites to the replacement named here.

## Streamed byte additions

Byte scans no longer need a page cache or repeated positional reads. Rust adds
`IOBase::pstream_bytes(position, batch_size)`,
`IOCursor::stream_bytes(batch_size)`, the lazy `ByteStream`, and the shared
64-KiB `DEFAULT_STREAM_BATCH_SIZE`. Python exposes `pstream_bytes` and
`stream_bytes`; JavaScript exposes `pstreamBytes` and `streamBytes`. All forms
yield bounded owned byte arrays lazily and fuse after their first error.

`IOBase.buffered` is now public in both extensions. Python accepts
`page_size`, `max_bytes`, and a TTL in seconds; JavaScript accepts `pageSize`,
`maxBytes`, and `ttlMs`. It replaces the options on the same native handle and
returns that handle, so repeated calls never stack caches.

Calling `open` on a generic `Holder` now promotes a recognized IPC, Parquet,
Avro, or Text representation into the matching native `Media` variant. Its
schema/footer cache is retained until `close`; closed handles continue to
derive fresh metadata on every operation. This is an optimization with no
new compatibility surface.

## Consuming conversion names

Project conversion APIs now use `from_*` for construction and `into_*` for
projection. The borrowed `to_*` aliases were removed; clone explicitly in Rust
when the source must be retained. Standard ecosystem protocols such as
`ToString::to_string`, `Vec::to_vec`, Serde helpers, and PyArrow/Polars methods
keep their upstream names.

| Removed | Replacement |
| --- | --- |
| Rust `DataType::to_arrow`, `to_arrow_schema`, `to_arrow_ffi` | `into_arrow`, `into_arrow_schema`, `into_arrow_ffi` |
| Rust `Field::to_arrow`, `to_arrow_ref`, `to_arrow_schema`, `to_arrow_exchange_schema`, `to_arrow_ffi` | the matching `into_*` method |
| Rust `DataType::to_value`, `Field::to_value` | `into_value` |
| Rust `DataType` / `Field` `to_json`, `to_yaml`, `to_toml` and `*_with_formatting` | the matching `into_*` method |
| Rust `DataType::to_scheme_compat`, `Field::to_scheme_compat` | `into_scheme_compat` |
| Rust `Metadata::to_json`, `Metadata::to_arrow`, `ProtocolMetadata::to_metadata` | `into_json`, `into_arrow`, `into_metadata` |
| Rust `Uri::to_url`, `to_urn`, `to_path`, `to_json`; `Url::to_uri`, `to_path`, `to_json`; `Urn::to_uri`, `to_json` | the matching `into_*` method |
| Rust `Timezone::to_local`, `Timezone::to_utc` | `into_local`, `into_utc` |
| Rust `StructScalar::to_arrow_scalar`, `TypedValue::to_arrow_array` | `into_arrow_scalar`, `into_arrow_array` |
| Rust WKB `Geometry::to_wkt` and `wkb::to_wkt` | `into_wkt` |
| Rust `Expression::to_json`, `Statement::to_json` | `into_json` |
| Rust Avro `to_single_object_vec`, `Schema::to_json`, `Schema::to_canonical_form` | `into_single_object_vec`, `into_json`, `into_canonical_form` |
| Rust Iceberg `to_json`, `to_v1_json`, and `PrimitiveType::to_data_type` projections | `into_json`, `into_v1_json`, `into_data_type` |
| Python `DataType.to_arrow`, `to_scheme_compat`, `to_json`, `to_yaml`, `to_toml`, `to_dict` | `into_arrow`, `into_scheme_compat`, `into_json`, `into_yaml`, `into_toml`, `into_dict` |
| Python `Field.to_arrow_schema`, `to_arrow`, `to_scheme_compat`, `to_json`, `to_yaml`, `to_toml`, `to_dict` | the matching `into_*` method |
| Python `Uri` / `Url` / `Urn` `to_url`, `to_urn`, `to_uri`, `to_path`, `to_json` | the matching `into_*` method |
| Python `MimeType.to_json`, `MediaType.to_json`, `Expression.to_json`, `Statement.to_json` | `into_json` |
| Python `Timezone.to_local`, `Timezone.to_utc` | `into_local`, `into_utc` |
| JavaScript `Expression.toJson`, `Statement.toJson` | `intoJson` |
| JavaScript `IOBase.toPath`; `Uri.toUrl` / `toUrn` / `toPath`; `Url.toUri` / `toPath`; `Urn.toUri` | the matching `into*` method |
| JavaScript `Timezone.toLocal`, `Timezone.toUtc` | `intoLocal`, `intoUtc` |

Structured codecs now make the returned representation explicit too:
`to_vec[_all]` became `into_bytes[_all]`, `to_writer[_all]` became
`into_writer[_all]`, and formatting variants follow the same spelling. Text
inputs use `from_utf8` or `from_bytes` instead of `from_str` or `from_slice`.

## Codec source intent

Codec reads no longer probe the filesystem to decide what a string means. Source intent is
carried by the argument type:

| Removed | Replacement |
| --- | --- |
| Python existing-file `str` source | `pathlib.Path(value)` (or another `os.PathLike`); every source `str` is document content |
| JavaScript existing-file string source | `pathToFileURL(value)` or a file descriptor; every source string is document content |

String destinations remain paths: a destination cannot be confused with document content. This
is a breaking source-read change with no existence-based fallback or compatibility probe.

## Generic values and natural text

`Value` now mirrors physical widths instead of collapsing them. Removed
variants and factories have no aliases:

| Removed | Replacement |
| --- | --- |
| `Decimal` / `decimal` | `D128` / `d128`; use `D256` / `d256` for a 256-bit coefficient |
| one generic float value | `F16`, `F32`, or `F64` |
| `Date` | `Date32` or `Date64` |
| `Time` | `Time32` or `Time64` |
| `Timestamp` and `DateTime` | `DateTime64(count, unit, timezone)`; use `Timezone::NAIVE` explicitly |
| `Duration` | `Duration32` or `Duration64` |
| insertion-shaped string-key maps used as rows | sorted `Value::Record(BTreeMap<name, Value>)` |

Every temporal carries a `TimeUnit` and a non-null `Timezone`. All values are
hashable and expose `as_bytes`, `as_utf8`, `as_json_bytes`, and
`as_json_utf8` (camelCase in JavaScript). Python and JavaScript expose exact
temporal components as `count` / `unit` / `zone` and decimal components as
`unscaled` / `scale`; unrelated kinds return `None` / `null`.

Container values now expose native persistent traversal in both extensions:
length/emptiness, iteration, indexed or keyed lookup, dotted paths, membership,
replacement, and removal. Every returned child remains a native `Value`, so an
exact integer, temporal, or decimal is never projected through a lossy host
number on the way. Replacement and removal return new values and leave the
source unchanged.

Field-directed canonicalization also preserves the declared signed or
unsigned integer width. A value accepted by an `Int8`, `Int16`, `Int32`,
`Int64`, `UInt8`, `UInt16`, `UInt32`, or `UInt64` field now returns that exact
physical `Value` variant rather than a widened integer.

Value-to-Arrow inference is now centralized in the core as
`inferred_scalar_field`, `inferred_array_field`, and `inferred_struct_field`.
The inferred names are uniformly `value`, `item`, and `row`; notably,
JavaScript array inference no longer uses `value` for the item Field. Duration
timezone validation likewise moved into native `duration32_in` /
`duration64_in`, so both bindings redirect instead of maintaining their own
NAIVE check. Python exposes the three Field results as `Value.into_field`,
`into_array_field`, and `into_struct_field`; JavaScript uses `intoField`,
`intoArrayField`, and `intoStructField`. These are direct core calls, not a
second binding-side inference path.

JSON, YAML, and TOML no longer emit or accept private tagged envelopes. They
write natural objects, arrays, strings, numbers, booleans, and null. Use a
declared `Field` while loading to recover exact decimal, binary, temporal, and
nested types. Python and JavaScript dumps return bytes by default, can return
UTF-8 text explicitly, or write the same result directly to a destination.

Field-directed JavaScript Struct loads now return named objects rather than
positional arrays, matching Python while Rust retains its canonical ordered
`Value::Sequence`. New `IOBase::read_value` / `write_value` methods expose the
same Value path directly on handles (`readValue` / `writeValue` in JavaScript),
including inferred gzip, zlib, and zstd content coding.

Python and JavaScript now expose the Rust Avro schema, object-container,
single-object, and lazy block codecs under `avro`. Their block iterators retain
compressed payloads until `rows` is requested, apply an optional reader schema
through one native resolution plan, and fuse after an error. Decode budgets are
the same core `Limits`: snake-case keywords in Python and one camel-case options
object in JavaScript. Only the explicit Rust `Resolution` plan type stays a
core-only implementation seam.

Python adds the format-inferred `yggdryl.codec` facade (`from_io`,
`from_stream`, `into_io`, `into_stream`), matching JavaScript's generic
`codec.from`, `fromStream`, `into`, and `intoStream`. Both infer from an
explicit format first, then a path suffix, then one native content sniff;
anonymous output defaults to JSON. JSON, YAML, and TOML now share nullable
depth/input/node/document limits and an `indent` option. These generic methods
redirect to the same native parsers and writers as the format modules.

Generic `RecordOptions` now also exposes Avro's validated block codec and
optional 16-byte synchronization marker. Python uses `block_codec` and
`sync_marker`; JavaScript uses `blockCodec` / `withBlockCodec` and
`syncMarker` / `withSyncMarker`. Invalid codec names, wrong-length markers,
and attempts to set them on another encoding fail in the core before a row
source is consumed.

| Removed JavaScript call | Replacement |
| --- | --- |
| `avro.loads(bytes, readerSchema)` | `avro.loads(bytes, { readerSchema })` |

## Bound statement runtime parity

Python and JavaScript now export `BoundStatement`. `Statement.bind` accepts a
struct field and optional named parameters, then redirects to the Rust
`Statement::bind_with` plan; no binding parses or evaluates the statement
independently. Both runtimes also expose statement `ordering` and `is_all` /
`isAll` inspection.

The bound plan exposes its schema, output field, projections, predicate,
ordering, limit, and all-row state. Arrow projection preserves record-batch,
table, and reader inputs, with a lazy reader path and one stream-wide limit.
Batch sorting is explicit because a globally ordered stream requires
materialization outside this API. These are additive APIs; no compatibility
aliases were introduced.

JavaScript temporal `Value` constructors now accept a native `Timezone`, a
timezone name, `null`, or no timezone. Omitted and `null` values infer
`Timezone.NAIVE`; `date32`/`date64` infer day/millisecond units, while the core
still validates which units and zones each temporal kind permits.

The binding-parity audit also added Parquet metadata reads to inferred
`IOBase` handles. Python exposes `read_parquet_statistics` and
`read_parquet_geospatial_statistics`; JavaScript exposes
`readParquetStatistics` and `readParquetGeospatialStatistics`. Both return the
same core-projected, native-language `Value` shape and reject non-Parquet media
with the core typed record error. The stateful `Parquet<H>` wrapper and its
open-session cache, free encoding functions, and Rust DTO accessors remain
intentional Rust-only seams; bindings do not duplicate those models.

## Field classes and declared record shape

| Removed | Replacement |
| --- | --- |
| Python `@record` / `records` module / `Record` | Python `@scalar` over an ordinary dataclass |
| schema-carrying Rust `Value::Record(Field, values)` / `Value::record` | `Value::Record(BTreeMap<name, Value>)` as a sorted named input, or `Value::Sequence` as the canonical row |
| Rust `Value::record_to_mapping` | use the new name-only `Value::Record`; struct `Field` canonicalization resolves it to schema order |
| datatype parser alias `record(...)` / `record<...>` | canonical `struct<...>` or SQL `row(...)` |
| Python `Class.FIELD` / `Class.into_struct_field()` | cached static `Class.field()` |
| JavaScript static `FIELD`, stored `intoStructField` Field, or `intoStructField()` method | actual static `get intoStructField()` getter, memoized by `intoField` |
| Python `field_of(value)` / `into_field(value)` | `field(value, name=None)` |
| Python `schema_field` / `schema_fields` | the native `Field` and its children |
| Rust `IORecordOptions::schema` | `IORecordOptions::field` |
| Rust `set_schema` / `require_schema` / `with_schema` | `set_field` / `require_field` / `with_field` |
| Rust `Ipc::schema`, `Parquet::schema`, `Avro::schema`, `Media::schema` | the matching `field()` method |
| Rust `Ipc::with_schema`, `Parquet::with_schema`, `Avro::with_schema`, `Media::with_schema` | the matching `with_field(...)` method |
| Rust `TextLineOptions::into_schema` | `TextLineOptions::into_field` |
| Rust `text::schema_from_pattern` | `TextLineOptions::with_pattern(pattern)?.into_field()` |
| Python `schema_from_pattern` / JavaScript `schemaFromPattern` | `field_from_pattern` / `fieldFromPattern` |
| Python `RecordOptions.schema` and flattened record keyword `schema=` | `RecordOptions.field`, supplied through the single `options=` value |
| JavaScript `RecordOptions.schema` / `withSchema` | `RecordOptions.field` / `withField` |
| Rust `arrow::schema_from_field` / `record_schema_from_arrow` / `record_schema_to_arrow` | `Field::into_arrow_schema` / `Field::from_arrow_schema` / `Field::into_arrow_exchange_schema` |
| Python decorated `Record.into_arrow_schema()` classmethod | `Class.field().into_arrow_schema()` |
| Parquet `read_schema` and stateful `read_field` | `read_arrow_schema` and stateful `field` |

The canonical dynamic conversion signature is documented on the [field page](field.md#converting-to-one-native-field).
Rust keeps its typed consuming accessors: `TypedField::into_field` and
`StructField::into_struct_field`.

The old schema-carrying record payload has no compatibility decoder. A row's schema is no longer
serialized once per row: use `Value::Sequence` for an already ordered row or the new sorted
`Value::Record` for name-directed input. JSON, YAML, and TOML write that record as a natural object;
no private record tag is emitted.

## Record writes

The handle-level write now names its intent. There are no aliases for the old
spellings:

| Removed | Replacement |
| --- | --- |
| Rust `IOBase::write_arrow_batch_reader(reader, options)` | `IOMedia::overwrite_arrow_reader(reader, options)` |
| Rust keyed `IOBase::write_arrow_batch_reader(reader, options.with_merge_by_names(keys))` | `IOMedia::merge_arrow_reader(reader, options.with_merge_by_names(keys))` |
| Rust `IOBase::append_arrow_batch_reader(reader, options)` | `IOMedia::append_arrow_reader(reader, options)` |
| Stateful `Ipc::write_batch_reader`, `Parquet::write_batch_reader`, `Avro::write_batch_reader`, `Media::write_batch_reader` | the matching `overwrite_arrow_reader(reader)` method |
| Free `ipc::write_batch_reader`, `parquet::write_batch_reader`, `avro::write_batch_reader` | the matching `overwrite_batch_reader` encoding seam |
| Python `read_arrow_batch_reader` / `read_arrow` | `read_arrow_reader` |
| Python `write_arrow_batch_reader` / `write_arrow` | the matching explicit `overwrite_arrow_reader`, `overwrite_arrow_table`, `overwrite_arrow_record_batch`, or `overwrite_records` method |
| Python keyed `write_arrow*` | the matching explicit `merge_arrow_*` method with keys in `options.merge_by_names` |
| Python `append_arrow_batch_reader` / `append_arrow` | the matching explicit `append_arrow_*` method |
| Python mode-less `write_records(records)` | `overwrite_records(records)` or `write_records(records, mode)` |
| JavaScript `readArrowBatchReader` / `readArrow` | `readArrowReader` |
| JavaScript `writeArrowBatchReader` / `writeArrow` | the matching explicit `overwriteArrowReader`, `overwriteArrowTable`, `overwriteArrowRecordBatch`, or `overwriteRecords` method |
| JavaScript keyed `writeArrow*` | the matching explicit `mergeArrow*` method with keys in `options.mergeByNames` |
| JavaScript `appendArrowBatchReader` / `appendArrow` | the matching explicit `appendArrow*` method |
| JavaScript mode-less `writeRecords(records)` | `overwriteRecords(records)` or `writeRecords(records, mode)` |
| JavaScript `BatchReader.toIpc()` / `toTable()` | consuming `intoIpc()` / `intoTable()` |

The Rust handle-level record contract now lives on `IOMedia`, with the final
shape-typed `read_arrow_reader` name and no legacy handle alias. Encoding modules
retain their precisely typed `read_batch_reader` implementation seams. The
bindings expose the matching `read_arrow_reader` / `readArrowReader` names. The write signatures, including
argument order and error semantics, are documented once under
[canonical record-write signatures](io.md#canonical-record-write-signatures).

Node's implicit native-record conversion chunk changed from an independent
1,024-row binding default to the core's shared 65,536-row
`DEFAULT_RECORD_BATCH_SIZE`. Set `options.batchSize = 1024` explicitly when
that older allocation bound is intentional; publication boundaries still come
only from `commitRowSize`.

`merge_by_names` no longer changes the meaning of another operation. Overwrite
and append reject non-empty merge keys; merge rejects an empty key list. Choose
the method first, then supply keys only for merge.

## Streamed publication cadence

`RecordOptions.commit_row_size` in Rust/Python and `RecordOptions.commitRowSize`
in JavaScript are the single optional publication boundary. `None`/`null` keeps the prior one-publication-at-EOF
behavior; positive `N` publishes complete `N`-row prefixes and the final
remainder. `batch_size` still bounds conversion and batch formation, not
publication. Code that previously treated a batch boundary as durable must set
`commit_row_size` explicitly.

Zero is not an alias for the default: it is rejected before a runtime iterator
or Arrow exporter is touched. Completed prefixes intentionally remain visible
after a later failure, so callers choosing a cadence also choose that recovery
semantics.

JavaScript async record sources use a resumable native session only when a
positive cadence is set; the session is internal and is not a public write
type. Unset cadence retains one-publication behavior through a bounded private
spool. Zero row/byte limits are synchronous and do not inspect even an async
source: append is a no-op and overwrite requires and publishes an empty
declared field.

## Generic write mode

`WriteMode` is the Rust enum with exactly `overwrite`, `append`, and `merge`.
The generic core dispatchers are:

```text
write_arrow_reader(reader, mode, options)
write_arrow_record_batch(batch, mode, options)
write_records(records, mode, options)
```

Python and JavaScript keep the runtime representation in the method name and
place the required mode immediately after the input. Python uses
`write_arrow_reader`, `write_arrow_table`, `write_arrow_record_batch`,
`write_records`, `write_pandas[_frame]`, and `write_polars[_frame]` with a
keyword-only `options=`. JavaScript uses `writeArrowReader`,
`writeArrowTable`, `writeArrowRecordBatch`, and `writeRecords`, followed by
one optional `options` value.

These are new required-mode APIs, not compatibility overloads. An omitted or
unknown mode fails before the input is inspected. Merge keys remain data for
the selected merge operation and never choose it.

## Media capability and dimensions

Record/schema/expression operations moved from `IOBase` to `IOMedia`; `IOBase`
implements that media contract and remains the only storage trait. Wrappers
now forward byte behavior with `delegate_iobase!` and media behavior with
`delegate_iomedia!`.

The new `is_io`, `row_size`, and `column_size` accessors are additions, not
aliases. Python exposes `is_io()`, `row_size`, and `column_size`; JavaScript
exposes `isIo()`, `rowSize`, and `columnSize`. Dimensions name the whole
logical media and ignore read projection/filter/limit settings. They use
format metadata where possible and cache only for an explicitly open handle.
