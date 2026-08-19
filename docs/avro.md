# Apache Avro

Read and write Avro - schemas, object containers, and schema resolution - as the shared [`Value`](generic.md) over any [`IOBase`](io.md) handle, with no Avro crate underneath.

!!! note "Rust only"
    The Python and JavaScript packages do not expose this module yet.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{MediaType, MimeType, Value, avro, json};

let schema = json::from_str(
    r#"{"type": "record", "name": "trade", "fields": [
        {"name": "symbol", "type": "string"},
        {"name": "quantity", "type": "long"},
        {"name": "price", "type": ["null", "double"], "default": null}
    ]}"#,
)?;
let rows = [
    json::from_str(r#"{"symbol": "AAPL", "quantity": 100, "price": 187.5}"#)?,
    json::from_str(r#"{"symbol": "MSFT", "quantity": 25, "price": null}"#)?,
];

let mut handle = Buffer::new();
handle.set_media_type(MediaType::new(MimeType::AVRO));
avro::write_container(&mut handle, &schema, &[("source", "docs")], &rows)?;

let container = avro::read_container(&handle)?;
assert_eq!(container.get("source"), Some("docs"));
assert_eq!(container.schema.kind(), "record");
assert_eq!(container.rows.len(), 2);
assert_eq!(
    container.rows[0].get_key_str("symbol").and_then(Value::as_str),
    Some("AAPL")
);
assert!(container.rows[1].get_key_str("price").is_some_and(Value::is_null));
```

An Avro object container is a self-describing file: its header carries the
writer's schema as JSON, so reading one needs nothing but the bytes.
`read_container` hands back the schema, the header metadata, and every row;
`write_container` takes the schema as its JSON [`Value`](generic.md), writes
that JSON into the header verbatim - so attributes this implementation does not
model, such as Iceberg's `field-id`, survive byte for byte - and encodes the
rows against it.

Rows cross the boundary in the JSON parser's vocabulary: a record is a mapping,
an array is a sequence, and a union carries the branch value directly - an
optional field reads as the value or `Null`, never as a wrapper naming the
branch.

## Schemas, canonical form, and fingerprints

```rust
use yggdryl::avro::Schema;

let schema = Schema::from_str(
    r#"{"type": "record", "name": "trade", "doc": "one fill", "fields": [
        {"name": "symbol", "type": "string"},
        {"name": "qty", "type": "long", "field-id": 2}
    ]}"#,
)?;

// The canonical form strips docs, defaults, and unknown attributes, and is
// what every implementation fingerprints.
assert_eq!(
    schema.to_canonical_form(),
    r#"{"name":"trade","type":"record","fields":[{"name":"symbol","type":"string"},{"name":"qty","type":"long"}]}"#
);

// The 64-bit Rabin fingerprint names the schema in caches and in the
// single-object framing; this value matches the reference implementations.
assert_eq!(schema.fingerprint().to_le_bytes()[0], 0xF5);

// The JSON the schema was parsed from round-trips verbatim, so the
// unmodeled `field-id` survives.
let text = String::from_utf8(yggdryl::json::to_vec(&schema.to_json())?)?;
assert!(text.contains("field-id"));
```

A [`Schema`] resolves namespaces, aliases, defaults, and recursive references
at parse time; a reference to a named type stays a reference, which is what
lets a recursive schema stay finite. `Schema::fingerprint` hashes the Parsing
Canonical Form with CRC-64-AVRO, so two schemas that differ only in
whitespace, attribute order, docs, or unknown attributes carry the same
fingerprint.

## Logical types decode as what they mean

```rust
use yggdryl::enums::TimeUnit;
use yggdryl::io::Buffer;
use yggdryl::{Timezone, Value, avro, json};

let schema = json::from_str(
    r#"{"type": "record", "name": "row", "fields": [
        {"name": "day", "type": {"type": "int", "logicalType": "date"}},
        {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
        {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                    "precision": 10, "scale": 2}}
    ]}"#,
)?;
let row = Value::from_mapping([
    (Value::from("day"), Value::Date(19_782)),
    (
        Value::from("at"),
        Value::Timestamp(1_700_000_000_000_000, TimeUnit::Microsecond, Timezone::UTC),
    ),
    (Value::from("price"), Value::Decimal(18_750, 2)),
])?;

let mut handle = Buffer::new();
avro::write_container(&mut handle, &schema, &[], &[row.clone()])?;
assert_eq!(avro::read_container(&handle)?.rows[0], row);
```

`date`, `time-millis`/`micros`, `timestamp-millis`/`micros`/`nanos`,
`local-timestamp-*`, `uuid` over string and fixed(16), `decimal` over bytes
and fixed, and `duration` are modeled, because the value model is typed: a
date is a calendar `Date`, a timestamp an instant in UTC, a decimal its exact
unscaled integer and scale. An annotation this implementation does not know -
or one whose attributes are invalid for its underlying type - degrades to the
underlying type, as the specification requires, never to an error. A decimal
wider than 38 digits keeps its raw bytes, because the value model holds a
decimal as a 128-bit integer; and a `duration` keeps its twelve bytes, because
the value model has no three-part month/day/millisecond interval.

## Reading with a different schema

```rust
use yggdryl::avro::Schema;
use yggdryl::io::Buffer;
use yggdryl::{Value, avro, json};

// The writer recorded three fields; the reader wants two - one renamed, one
// promoted, plus a field the writer never knew, filled from its default.
let writer = json::from_str(
    r#"{"type": "record", "name": "trade", "fields": [
        {"name": "symbol", "type": "string"},
        {"name": "qty", "type": "int"},
        {"name": "venue", "type": "string"}
    ]}"#,
)?;
let reader = Schema::from_str(
    r#"{"type": "record", "name": "trade", "fields": [
        {"name": "quantity", "aliases": ["qty"], "type": "long"},
        {"name": "note", "type": "string", "default": "none"}
    ]}"#,
)?;

let mut handle = Buffer::new();
avro::write_container(
    &mut handle,
    &writer,
    &[],
    &[json::from_str(r#"{"symbol": "AAPL", "qty": 100, "venue": "XNAS"}"#)?],
)?;

let container = avro::read_container_resolved(&handle, &reader)?;
assert_eq!(
    container.rows[0].get_key_str("quantity").and_then(Value::as_i64),
    Some(100),
    "matched through the alias, promoted int to long"
);
assert_eq!(
    container.rows[0].get_key_str("note").and_then(Value::as_str),
    Some("none")
);
assert_eq!(container.rows[0].len(), 2, "unwanted writer fields are skipped");
```

`read_container_resolved` compiles the specification's resolution matrix into
a [`Resolution`] once per (writer, reader) pair and executes it per row:
fields match by name or reader alias in any order, `int` promotes to `long`,
`float`, or `double` (and `long` and `float` upward likewise), `string` and
`bytes` interchange, enum symbols map with the reader's default as fallback,
and unions resolve branch by branch. A writer field the reader does not name
is skipped without being decoded - length-prefixed values jump by their
prefix, and an array or map block written in the size-carrying form jumps as
one seek - which is what makes projection cheap. An illegal resolution is
refused when the plan is built, naming both sides and the field path; a union
branch the reader cannot accept fails only when a datum actually takes it,
which is the specification's rule.

## Streaming a large container

```rust
use yggdryl::io::Buffer;
use yggdryl::{Value, avro, json};

let schema = json::from_str(r#"{"type": "record", "name": "row", "fields": [
    {"name": "id", "type": "long"}]}"#)?;
let rows: Vec<Value> = (0..3)
    .map(|id| Value::from_mapping([(Value::from("id"), Value::from(id))]))
    .collect::<Result<_, _>>()?;

let mut handle = Buffer::new();
avro::write_container(&mut handle, &schema, &[], &rows)?;

let mut blocks = avro::read_blocks(&handle)?;
assert_eq!(blocks.schema().kind(), "record");
while let Some(block) = blocks.next_block()? {
    // A block is handed back still compressed; decoding is the caller's
    // choice, so skipping a block costs nothing.
    assert_eq!(block.rows()?.len() as u64, block.count());
}
```

`read_blocks` iterates a container over nothing but `pread`, so it works on
any handle without holding the file in memory. Each `Block` arrives still
compressed with its declared row count; `rows` decompresses and decodes it,
`rows_resolved` does the same through a [`Resolution`], and not calling either
skips the block entirely. `read_container` stays the fast case for small
self-describing files - an Iceberg manifest describes files, not rows, so it
is small by construction.

## Single-object encoding

```rust
use yggdryl::avro::Schema;
use yggdryl::{Value, avro};

let schema = Schema::from_str(r#"{"type": "record", "name": "tick", "fields": [
    {"name": "price", "type": "double"}]}"#)?;
let value = Value::from_mapping([(Value::from("price"), Value::from(187.5))])?;

let framed = avro::to_single_object_vec(&schema, &value)?;
assert_eq!(&framed[..2], &[0xC3, 0x01], "the single-object marker");
assert_eq!(avro::from_single_object_slice(&framed, &schema)?, value);

// A frame from a different schema is refused naming both fingerprints.
let other = Schema::from_str("\"long\"")?;
let message = avro::from_single_object_slice(&framed, &other)
    .unwrap_err()
    .to_string();
assert!(message.contains("fingerprint"));
```

A message system that cannot afford a container header per record frames each
datum as `C3 01`, the writer schema's Rabin fingerprint in little-endian
order, and the body. The fingerprint is how a receiver picks the writer schema
out of a store - and the natural key for caching a [`Resolution`].

## The record surface

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("symbol"),
])?
.required_field("row");
let arrow_schema = schema.to_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![
        Arc::new(Int64Array::from(vec![1, 2])),
        Arc::new(StringArray::from(vec![Some("AAPL"), None])),
    ],
)?;

// The name decides the encoding; nothing else in the call changes.
let mut handle =
    Buffer::new().with_media_type(Url::from_str("file:///trades.avro")?.media_type());
let options = handle.record_options()?;
handle.write_arrow_batch_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch]), &options)?;
assert_eq!(handle.read_arrow_batch_reader(&options)?.count(), 1);
```

Avro answers the same three record methods as every other encoding: a handle
whose media type says `avro` reads and writes Arrow batches with no format
argument anywhere. Decoding is columnar - one builder per leaf, appended per
record, with no intermediate `Value` tree on that path - and a declared schema
becomes the encoding's own projection: an unselected top-level column's bytes
are *skipped*, not decoded, so a projection saves decode, allocation, and
those bytes. What it cannot save is reading the row, because Avro interleaves
columns per record; Parquet, whose column chunks are separately addressable,
is where a projection also skips reading.

`avro::Avro` is the stateful form - handle, options, and a schema cache that
[`IOBase::open`](io.md) fills and `close` releases - and `avro::AvroOptions`
adds two settings to the shared surface: the block codec name and an optional
fixed synchronization marker for byte-reproducible writes. A union wider than
`null` plus one branch, a recursive schema, or a datatype Avro cannot spell is
refused by name on this surface; the `Value`-level functions above have no
such limits. Avro compresses inside its blocks, so - like Parquet and unlike
IPC - a handle declaring an outer content coding such as `trades.avro.gz` is
rejected rather than double-compressed.

## Codecs and limits

Blocks are decompressed with the codec the header names: `null`, `deflate`,
and `zstandard` map onto the crate's own [`Codec`](enums.md) implementations,
and `snappy` - raw Snappy followed by a big-endian CRC-32 of the uncompressed
block - decodes in builds carrying the `parquet` feature, which is what
already compiles the Snappy code. Any other name, `bzip2` and `xz` among
them, is refused naming it and listing what this build implements.

Every reading entry point has a `_with_limits` form taking the crate's
[`Limits`](text.md): input bytes bound the container and each decompressed
block, depth bounds schema and datum nesting, and the node budget bounds what
one datum may allocate - so a hostile container is a typed error, never an
allocation the process dies of. Malformed input carries the byte position at
or immediately after the failure, in the same shape every other codec in the
crate reports.

[`Schema`]: #schemas-canonical-form-and-fingerprints
[`Resolution`]: #reading-with-a-different-schema

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/avro.ipynb){ download }.

<!-- /notebooks -->
