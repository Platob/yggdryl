# Apache Avro

Read and write Avro as streamed Arrow batches first, then use its more flexible
schema, object-container, and single-value operations over the shared
[`Scalar`](generic.md). Every path works over any [`IOBase`](io.md) handle, with
no Avro crate underneath.

!!! note "Two surfaces"
    The handle-level Arrow record surface below is available in Rust, Python,
    and JavaScript. Both bindings also expose the native `Schema`, whole
    object-container, single-object, and lazy compressed-block operations
    through their natural values. The explicit compiled `Resolution` type is
    Rust-only; binding `reader_schema` options compile and reuse it internally.

## Arrow batch reads and writes

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOBase, IOMedia};
    use yggdryl::{DataType, Url};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = |ids: Vec<i64>, venues: Vec<Option<&str>>| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(venues)),
            ],
        )
    };

    // The name decides Avro; the methods name the write intent.
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.avro")?.media_type());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch(vec![1, 2], vec![Some("XNAS"), Some("XNYS")])?]),
        &options,
    )?;
    handle.append_arrow_reader(
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch(vec![3], vec![Some("XLON")])?]),
        &options,
    )?;
    handle.merge_arrow_reader(
        yggdryl::arrow::batch_reader(Arc::clone(&schema), [batch(vec![2, 4], vec![Some("XPAR"), None])?]),
        &options.clone().with_merge_by_names(["id"]),
    )?;

    let rows = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .sum::<Result<usize, _>>()?;
    assert_eq!(rows, 4);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    batch = lambda ids, venues: pa.record_batch(
        {"id": ids, "venue": venues}, schema=schema
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.avro")
    handle.overwrite_arrow_batch(batch([1, 2], ["XNAS", "XNYS"]))
    handle.append_arrow_batch(batch([3], ["XLON"]))

    merging = handle.record_options()
    merging.merge_by_names = ["id"]
    handle.merge_arrow_batch(batch([2, 4], ["XPAR", None]), options=merging)

    assert handle.read_arrow_reader().read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const rows = (ids, venues) => new arrow.Table({
      id: arrow.vectorFromArray(ids.map(BigInt), new arrow.Int64()),
      venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
    })

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.avro'))
    handle.overwriteArrowTable(rows([1, 2], ['XNAS', 'XNYS']))
    handle.appendArrowTable(rows([3], ['XLON']))
    handle.mergeArrowTable(
      rows([2, 4], ['XPAR', null]),
      handle.recordOptions().withMergeByNames(['id']),
    )

    assert.equal(handle.readArrowReader().intoTable().numRows, 4)
    fs.rmSync(root, { recursive: true, force: true })
    ```

Avro answers the same read and explicit overwrite, append, and merge intents as
every other encoding. Their [canonical signatures and streaming
rules](io.md#canonical-record-write-signatures) apply to a handle whose media
type says `avro`, with no format argument anywhere. Decoding is columnar - one builder per leaf, appended per
record, with no intermediate `Scalar` tree on that path - and a declared schema
becomes the encoding's own projection: an unselected top-level column's bytes
are *skipped*, not decoded, so a projection saves decode, allocation, and
those bytes. What it cannot save is reading the row, because Avro interleaves
columns per record; Parquet, whose column chunks are separately addressable,
is where a projection also skips reading.

### Measured batch operations

The read fixture contains 65,536 rows and four columns. The write fixture contains 4,096 rows;
Criterion prepares the stored side for append and keyed merge outside the timer. Keyed merge is
the upsert operation: matching `id` rows are updated and misses are inserted.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 26.0 ms | 2.52M rows/s |
| `overwrite_arrow_reader` | 4,096 | 6.10 ms | 671k rows/s |
| `append_arrow_reader` | 4,096 | 12.9 ms | 318k rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 11.4 ms | 358k rows/s |

These are Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23). Regenerate them on the deployment host with
`io_dimensions/avro/read_rows` and `io_write_stateful/avro`; the format-level comparisons remain
in [Benchmarks](#benchmarks).

### Dimensions and opened sessions

`row_size` walks block counts and encoded lengths, jumps each payload positionally, and validates
its synchronization marker without allocating, decompressing, or decoding rows. `column_size`
reads only the header schema. Both describe the whole container, ignoring selections, filters, and
limits. Closed calls derive fresh metadata; `open` retains the inferred Avro wrapper and caches its
schema and dimensions until `close`. Writes invalidate the cache.

The 65,536-row fixture measured fresh/opened `row_size` at 20.6 us/83.6 ns,
fresh/opened `column_size` at 22.3 us/87.8 ns, and fresh/opened `read_arrow_field` at
101 us/72.5 us. Regenerate with
`cargo bench -p yggdryl --bench io --all-features -- io_dimensions/avro`.

`avro::Avro` is the stateful form - handle, options, and a metadata cache that
[`IOBase::open`](io.md) fills and `close` releases - and `avro::AvroOptions`
adds two settings to the shared surface: the block codec name and an optional
fixed synchronization marker for byte-reproducible writes. A union wider than
`null` plus one branch, a recursive schema, or a datatype Avro cannot spell is
refused by name on this surface; the `Scalar`-level functions below have no
such limits. Avro compresses inside its blocks, so - like Parquet and unlike
IPC - a handle declaring an outer content coding such as `trades.avro.gz` is
rejected rather than double-compressed.

### Block encoding options

The generic `RecordOptions` exposes those Avro-only settings without downcasting.
Codec names are validated through the same core vocabulary the writer dispatches
before a row source is pulled: `null`, `deflate`, `zstandard`, and `snappy` when
the build includes its compression support. A fixed marker is either absent or
exactly 16 bytes; absence generates a fresh marker for each write. Setting either
property on options for another encoding is a typed record error.

=== "Rust"

    ```rust
    use yggdryl::generic::RecordOptions;
    use yggdryl::MimeType;

    let mut options = RecordOptions::for_mime_type(&MimeType::AVRO)?;
    assert_eq!(options.avro_block_codec(), Some("deflate"));
    assert_eq!(options.avro_sync_marker(), None);

    options.set_avro_block_codec("zstandard")?;
    options.set_avro_sync_marker(Some(b"0123456789abcdef"))?;
    assert_eq!(options.avro_sync_marker(), Some(b"0123456789abcdef"));
    ```

=== "Python"

    ```python
    from yggdryl import RecordOptions

    options = RecordOptions("trades.avro")
    assert options.block_codec == "deflate"
    assert options.sync_marker is None

    options.block_codec = "zstandard"
    options.sync_marker = b"0123456789abcdef"
    assert options.sync_marker == b"0123456789abcdef"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { RecordOptions } = require('yggdryl')

    const options = RecordOptions.from('trades.avro')
      .withBlockCodec('zstandard')
      .withSyncMarker(Buffer.from('0123456789abcdef'))

    assert.equal(options.blockCodec, 'zstandard')
    assert.deepEqual(options.syncMarker, Buffer.from('0123456789abcdef'))
    ```

The generic enum redirects these operations without downcasting or allocation. A local Windows
x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23) measured a codec
read at 9.65 ns and setting both codec and fixed marker at 49.8 ns. These are Criterion point
estimates; regenerate them on the deployment host with:

```console
cargo bench -p yggdryl --bench io --all-features -- "io_dimensions/avro/options"
```

## Flexible Scalar containers and schema methods

!!! note "Native Scalar surface"
    Python exposes this layer as `avro.Schema`, `loads` / `dumps`, and
    `loads_single` / `dumps_single`; JavaScript uses `Schema`, `loads` /
    `dumps`, and `loadsSingle` / `dumpsSingle`. Their natural host values cross
    through the same core [`Scalar`](generic.md). Their `blocks` iterator keeps
    compressed blocks lazy; the explicit `Resolution` object remains Rust-only.

=== "Rust"

    ```rust
    use yggdryl::io::Buffer;
    use yggdryl::{Scalar, avro, json};

    let schema = json::from_utf8(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"symbol","type":"string"},
            {"name":"quantity","type":"long"}]}
        "#,
    )?;
    let rows = [
        json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?,
        json::from_utf8(r#"{"symbol":"MSFT","quantity":25}"#)?,
    ];
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[("source", "docs")], &rows)?;

    let decoded = avro::read_container(&handle)?;
    assert_eq!(decoded.get("source"), Some("docs"));
    assert_eq!(decoded.rows.len(), 2);
    assert_eq!(
        decoded.rows[0].get_key_str("symbol").and_then(Scalar::as_utf8),
        Some("AAPL")
    );
    ```

=== "Python"

    ```python
    from yggdryl import avro

    schema = {
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "quantity", "type": "long"},
        ],
    }
    encoded = avro.dumps(
        [{"symbol": "AAPL", "quantity": 100}, {"symbol": "MSFT", "quantity": 25}],
        schema,
        metadata={"source": "docs"},
    )
    decoded = avro.loads(encoded)

    assert decoded.metadata == {"source": "docs"}
    assert decoded.rows[0] == {"quantity": 100, "symbol": "AAPL"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = {
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'quantity', type: 'long' },
      ],
    }
    const encoded = avro.dumps(
      [{ symbol: 'AAPL', quantity: 100 }, { symbol: 'MSFT', quantity: 25 }],
      schema,
      { source: 'docs' },
    )
    const decoded = avro.loads(encoded)

    assert.deepEqual(decoded.metadata, { source: 'docs' })
    assert.deepEqual(decoded.rows[0], { quantity: 100, symbol: 'AAPL' })
    ```

An Avro object container is a self-describing file: its header carries the
writer's schema as JSON, so reading one needs nothing but the bytes.
`read_container` hands back the schema, the header metadata, and every row;
`write_container` takes the schema as its JSON [`Scalar`](generic.md), writes
that JSON into the header verbatim - so attributes this implementation does not
model, such as Iceberg's `field-id`, survive byte for byte - and encodes the
rows against it.

Rows cross the boundary in the JSON parser's vocabulary: a record is a mapping,
an array is a sequence, and a union carries the branch value directly - an
optional field reads as the value or `Null`, never as a wrapper naming the
branch.

## Schemas, canonical form, and fingerprints

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;

    let schema = Schema::from_str(
        r#"{"type": "record", "name": "trade", "doc": "one fill", "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2}
        ]}"#,
    )?;

    assert!(!schema.clone().into_canonical_form().contains("doc"));
    assert_eq!(schema.fingerprint().to_le_bytes()[0], 0xF5);
    let text = String::from_utf8(yggdryl::json::into_bytes(&schema.into_json())?)?;
    assert!(text.contains("field-id"));
    ```

=== "Python"

    ```python
    from yggdryl import avro

    document = {
        "type": "record",
        "name": "trade",
        "doc": "one fill",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2},
        ],
    }
    schema = avro.Schema(document)

    assert "doc" not in schema.into_canonical_form()
    assert schema.fingerprint().to_bytes(8, "little")[0] == 0xF5
    assert schema.into_json()["fields"][1]["field-id"] == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = new avro.Schema({
      type: 'record',
      name: 'trade',
      doc: 'one fill',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'qty', type: 'long', 'field-id': 2 },
      ],
    })

    assert.ok(!schema.canonicalForm.includes('doc'))
    assert.equal(Number(schema.fingerprint & 0xffn), 0xf5)
    assert.equal(schema.intoJSON().fields[1]['field-id'], 2)
    ```

A [`Schema`] resolves namespaces, aliases, defaults, and recursive references
at parse time; a reference to a named type stays a reference, which is what
lets a recursive schema stay finite. `Schema::fingerprint` hashes the Parsing
Canonical Form with CRC-64-AVRO, so two schemas that differ only in
whitespace, attribute order, docs, or unknown attributes carry the same
fingerprint.

A fingerprint is a wire-layout identifier, not the schema object's equality
identity. Parsing Canonical Form deliberately strips logical annotations,
aliases, and defaults even though those change the native value produced or
reader resolution. Schema equality, total ordering, and `stable_hash` therefore
use the complete retained JSON document. The bindings redirect their
`equals`/comparison/hash protocols to that core identity; two schemas may have
the same Avro fingerprint while remaining distinct Yggdryl schema values.

## Logical types decode as what they mean

=== "Rust"

    ```rust
    use yggdryl::generic::TimeUnit;
    use yggdryl::io::Buffer;
    use yggdryl::{Timezone, Scalar, avro, json};

    let schema = json::from_utf8(
        r#"{"type": "record", "name": "row", "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}}
        ]}"#,
    )?;
    let row = Scalar::from_record([
        ("day", Scalar::Date32(19_782, TimeUnit::Day, Timezone::NAIVE)),
        ("at", Scalar::DateTime64(
            1_700_000_000_000_000,
            TimeUnit::Microsecond,
            Timezone::UTC,
        )),
        ("price", Scalar::D128(18_750, 2)),
    ])?;

    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[], &[row.clone()])?;
    assert_eq!(avro::read_container(&handle)?.rows[0], row);
    ```

=== "Python"

    ```python
    from datetime import date, datetime, timezone
    from decimal import Decimal

    from yggdryl import avro

    schema = {
        "type": "record",
        "name": "row",
        "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}},
        ],
    }
    row = {
        "day": date(2024, 2, 29),
        "at": datetime(2023, 11, 14, 22, 13, 20, tzinfo=timezone.utc),
        "price": Decimal("187.50"),
    }

    decoded = avro.loads(avro.dumps([row], schema)).rows[0]
    assert decoded == row
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, avro } = require('yggdryl')

    const decimal = {
      type: 'bytes',
      logicalType: 'decimal',
      precision: 10,
      scale: 2,
    }
    const value = Scalar.decimal(18750n, 2)
    const decoded = avro.loadsSingle(avro.dumpsSingle(value, decimal), decimal)

    assert.ok(decoded instanceof Scalar)
    assert.equal(decoded.kind, 'd128')
    assert.equal(decoded.unscaled, 18750n)
    assert.equal(decoded.scale, 2)
    ```

`date`, `time-millis`/`micros`, `timestamp-millis`/`micros`/`nanos`,
`local-timestamp-*`, `uuid` over string and fixed(16), `decimal` over bytes
and fixed, and `duration` are modeled, because the value model is typed: a
date is `Date32`, a timestamp is `DateTime64` with `UTC`, and a decimal keeps
its exact coefficient and scale. An annotation this implementation does not know -
or one whose attributes are invalid for its underlying type - degrades to the
underlying type, as the specification requires, never to an error. A decimal
wider than the encoding's supported 38 digits keeps its raw bytes; an Avro
`duration` also keeps its twelve bytes because it is a three-part
month/day/millisecond interval, not one elapsed count.

## Reading with a different schema

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;
    use yggdryl::io::Buffer;
    use yggdryl::{Scalar, avro, json};

    let writer = json::from_utf8(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"symbol","type":"string"},
            {"name":"qty","type":"int"},
            {"name":"venue","type":"string"}]}"#,
    )?;
    let reader = Schema::from_str(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"quantity","aliases":["qty"],"type":"long"},
            {"name":"note","type":"string","default":"none"}]}"#,
    )?;
    let row = json::from_utf8(r#"{"symbol":"AAPL","qty":100,"venue":"XNAS"}"#)?;
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &writer, &[], &[row])?;

    let decoded = avro::read_container_resolved(&handle, &reader)?;
    assert_eq!(
        decoded.rows[0].get_key_str("quantity").and_then(Scalar::as_i64),
        Some(100),
    );
    assert_eq!(decoded.rows[0].len(), 2, "unwanted writer fields are skipped");
    ```

=== "Python"

    ```python
    from yggdryl import avro

    writer = {
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "int"},
            {"name": "venue", "type": "string"},
        ],
    }
    reader = avro.Schema({
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "quantity", "aliases": ["qty"], "type": "long"},
            {"name": "note", "type": "string", "default": "none"},
        ],
    })
    encoded = avro.dumps([{"symbol": "AAPL", "qty": 100, "venue": "XNAS"}], writer)

    assert avro.loads(encoded, reader_schema=reader).rows == [
        {"note": "none", "quantity": 100}
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const writer = {
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'qty', type: 'int' },
        { name: 'venue', type: 'string' },
      ],
    }
    const reader = new avro.Schema({
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'quantity', aliases: ['qty'], type: 'long' },
        { name: 'note', type: 'string', default: 'none' },
      ],
    })
    const encoded = avro.dumps(
      [{ symbol: 'AAPL', qty: 100, venue: 'XNAS' }],
      writer,
    )

    assert.deepEqual(avro.loads(encoded, { readerSchema: reader }).rows, [
      { note: 'none', quantity: 100 },
    ])
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

=== "Rust"

    ```rust
    use yggdryl::io::Buffer;
    use yggdryl::{Scalar, avro, json};

    let schema = json::from_utf8(r#"{"type":"record","name":"row","fields":[
        {"name":"id","type":"long"}]}"#)?;
    let rows: Vec<Scalar> = (0..3)
        .map(|id| Scalar::from_record([("id", Scalar::from(id))]))
        .collect::<Result<_, _>>()?;
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[], &rows)?;

    let mut blocks = avro::read_blocks(&handle)?;
    assert_eq!(blocks.schema().kind(), "record");
    while let Some(block) = blocks.next_block()? {
        assert_eq!(block.rows()?.len() as u64, block.count());
    }
    ```

=== "Python"

    ```python
    from yggdryl import avro

    schema = {
        "type": "record",
        "name": "row",
        "fields": [{"name": "id", "type": "long"}],
    }
    stream = avro.blocks(avro.dumps([{"id": 1}, {"id": 2}, {"id": 3}], schema))

    assert stream.schema.kind == "record"
    block = next(stream)
    assert block.count == len(block.rows())
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = {
      type: 'record',
      name: 'row',
      fields: [{ name: 'id', type: 'long' }],
    }
    const stream = avro.blocks(avro.dumps([{ id: 1 }, { id: 2 }, { id: 3 }], schema))

    assert.equal(stream.schema.kind, 'record')
    const block = stream.next().value
    assert.equal(block.count, BigInt(block.rows().length))
    ```

`read_blocks` iterates a container over nothing but `pread`, so it works on
any handle without holding the file in memory. Python and JavaScript accept an
already-held byte value and copy it into the owning native handle once; block
payloads remain compressed and row decoding stays lazy. Their `avro.blocks`
iterator is fused after its first error. Each `Block` arrives still
compressed with its declared row count; `rows` decompresses and decodes it,
`rows_resolved` does the same through a [`Resolution`], and not calling either
skips the block entirely. `read_container` stays the fast case for small
self-describing files - an Iceberg manifest describes files, not rows, so it
is small by construction.

## Single-object encoding

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;
    use yggdryl::{Scalar, avro};

    let schema = Schema::from_str(r#"{"type":"record","name":"tick","fields":[
        {"name":"price","type":"double"}]}"#)?;
    let value = Scalar::from_record([("price", Scalar::from(187.5))])?;
    let framed = avro::into_single_object_vec(&schema, &value)?;

    assert_eq!(&framed[..2], &[0xC3, 0x01]);
    assert_eq!(avro::from_single_object_slice(&framed, &schema)?, value);
    ```

=== "Python"

    ```python
    from yggdryl import avro

    schema = avro.Schema({
        "type": "record",
        "name": "tick",
        "fields": [{"name": "price", "type": "double"}],
    })
    framed = avro.dumps_single({"price": 187.5}, schema)

    assert framed[:2] == b"\xc3\x01"
    assert avro.loads_single(framed, schema) == {"price": 187.5}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = new avro.Schema({
      type: 'record',
      name: 'tick',
      fields: [{ name: 'price', type: 'double' }],
    })
    const framed = avro.dumpsSingle({ price: 187.5 }, schema)

    assert.deepEqual(framed.subarray(0, 2), Buffer.from([0xc3, 0x01]))
    assert.deepEqual(avro.loadsSingle(framed, schema), { price: 187.5 })
    ```

A message system that cannot afford a container header per record frames each
datum as `C3 01`, the writer schema's Rabin fingerprint in little-endian
order, and the body. The fingerprint is how a receiver picks the writer schema
out of a store - and the natural key for caching a [`Resolution`].

## Codecs and limits

=== "Rust"

    ```rust
    use yggdryl::io::Buffer;
    use yggdryl::{Limits, Scalar, avro, json};

    let schema = json::from_utf8(r#""long""#)?;
    let mut bytes = Buffer::new();
    avro::write_container(&mut bytes, &schema, &[], &[Scalar::I64(7)])?;

    let limits = Limits::new(8, 1_024, 8, 1);
    assert_eq!(avro::read_container_with_limits(&bytes, limits)?.rows, [Scalar::I64(7)]);
    ```

=== "Python"

    ```python
    from yggdryl import avro

    encoded = avro.dumps([7], '"long"')
    decoded = avro.loads(
        encoded,
        max_depth=8,
        max_input_bytes=1_024,
        max_nodes=8,
    )

    assert decoded.rows == [7]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const encoded = avro.dumps([7], '"long"')
    const decoded = avro.loads(encoded, {
      maxDepth: 8,
      maxInputBytes: 1024,
      maxNodes: 8,
    })

    assert.deepEqual(decoded.rows, [7])
    ```

Blocks are decompressed with the codec the header names: `null`, `deflate`,
and `zstandard` map onto the crate's own [`Codec`](generic.md) implementations,
and `snappy` - raw Snappy followed by a big-endian CRC-32 of the uncompressed
block - decodes in builds carrying the `parquet` feature, which is what
already compiles the Snappy code. Any other name, `bzip2` and `xz` among
them, is refused naming it and listing what this build implements.

Every Rust reading entry point has a `_with_limits` form taking the crate's
[`Limits`](text.md); Python uses matching snake-case keywords and JavaScript
uses the camel-case options shown above. Input bytes bound the container and each decompressed
block, depth bounds schema and datum nesting, and the node budget bounds rows and what one datum
may allocate. Opening the mandatory header does not consume that row budget; its structure keeps
the core safety floor while the caller's byte and depth bounds still apply. Lazy block iteration
therefore reports a low node limit at the first row that exceeds it, after the header has opened.
A hostile container is a typed error, never an allocation the process dies of. Malformed input
carries the byte position at or immediately after the failure, in the same shape every other codec
in the crate reports.

[`Schema`]: #schemas-canonical-form-and-fingerprints
[`Resolution`]: #reading-with-a-different-schema

## Benchmarks

The JavaScript raw-codec path below uses one 4,580-byte container of 1,000
three-column rows, with fixtures outside the loops. From `npm run bench:codec`
on Node 24.18.0, an x86-64 Windows release build (AMD Ryzen 5 150):

| operation | ms/op |
| --- | ---: |
| schema parse / canonical form | 0.136 / 0.003 |
| container decode / resolve / encode | 13.075 / 10.549 / 18.667 |
| first compressed block / decode / resolve | 0.054 / 13.833 / 11.979 |
| single-object decode / encode | 0.018 / 0.087 |

The first-block number includes header parsing and the first lazy `next`, but
not row decompression. The block decode and resolution rows measure that work
separately.

### Against fastavro and PyIceberg, on identical bytes

`python scripts/bench_avro_baseline.py` has the Rust half write one deterministic
ten-thousand-entry Iceberg manifest (112,246 bytes, statistics included) and then times three
implementations over that exact file - so the rows below are readers of identical bytes, not of
similar fixtures. From one containerized x86_64 Linux run (rustc stable **release** build,
CPython 3.11.15, fastavro 1.12.2, pyiceberg 0.11.1):

```text
fastavro 1.12.2:                    67,719 entries/s best (147.7 ms best of 7)
pyiceberg 0.11.1:                   46,790 entries/s best (213.7 ms best of 7)
yggdryl full (release):            101,937 entries/s best ( 98.1 ms best of 7)
yggdryl plan_stats (release):      203,252 entries/s best ( 49.2 ms best of 7)
yggdryl plan_identity (release):   438,596 entries/s best ( 22.8 ms best of 7)
```

`full` is `read_manifest`, decoding every field the way the other two readers do.
`plan_stats` is `read_manifest_for_plan(handle, true)` - what a *filtered* scan runs - which keeps
the value counts, null counts, and bounds that pruning consults and skips the rest as bytes.
`plan_identity` is the unfiltered planning read, which keeps only file identity, partition tuple,
and sizes. The gap between the three is the measured worth of the skip routines: on this manifest
the planning path is 2.1x the full decode with statistics kept and 4.4x without, and the ratios
hold at 1,000 and 100,000 entries (`manifest/decode_full`, `manifest/decode_plan_with_stats`,
`manifest/decode_plan_identity_only` in the `iceberg` target).

From the same machine, the `avro` target's own groups
(`cargo bench --bench avro --features "parquet iceberg"`):

- **Types** (`codec/avro_types`, 10,000 rows each): primitives decode at ~2.9M rows/s and encode
  at ~3.5M rows/s; two-string rows at ~3.3M rows/s decode; 18-digit decimals at ~6.7M rows/s;
  the deeply nested family (array of records of maps) at ~620K rows/s. The single-object varint
  floor sits at ~57-65 ns per framed datum.
- **Codec x block size** (`codec/avro_blocks`, 65,536 three-column rows): decode throughput is
  nearly flat from 1,024 to 65,536 rows per block for every codec - the sweet spot is "anything
  above ~1,000 rows"; below that the per-block header and sync overhead starts to show. On this
  payload snappy decodes at ~20 MiB/s of encoded bytes, deflate at ~12, zstandard at ~9.6, and
  the null codec at ~38 (its bytes are bigger, so the row rate is what to compare).
- **Projection** (`codec/avro_projection`, 40 columns, null codec so the skip itself is visible):
  reading 3 of 40 columns takes 6.4 ms against 9.4 ms for all 40 over 8,192 rows. Avro
  interleaves columns per record, so a projection can never skip *reading* a row - the saving is
  the decode and allocation of the 37 skipped columns, jumped by their length prefixes.
- **Resolution** (`codec/avro_resolution`): compiling a five-field plan costs ~533 ns once;
  executing it per row is *cheaper* than the direct decode on this shape (4.12 ms against 4.74 ms
  for 10,000 rows) because the plan skips two writer columns the reader never wanted - the
  per-record cost of resolving is not just near zero, it can be negative.
