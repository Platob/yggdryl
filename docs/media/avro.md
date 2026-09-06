# Apache Avro

Owns Avro as streamed Arrow batches over any [`IOBase`](../holder/index.md) handle and the [`Scalar`](../types/scalar.md)-level schema, container, block, and single-object operations, with no Avro crate underneath.

## Contract

| | |
| --- | --- |
| Owns | `avro::Avro` (stateful handle form), `avro::AvroOptions`, `Schema`, `Resolution`, `read_container`/`write_container`, `read_blocks`, single-object framing |
| Bindings | Python `yggdryl.media.avro`: `Schema`, `loads`/`dumps`, `loads_single`/`dumps_single`, `blocks`; JavaScript `avro`: `Schema`, `loads`/`dumps`, `loadsSingle`/`dumpsSingle`, `blocks` |
| Rust only | `Resolution`; a binding `reader_schema` option compiles and reuses it internally |
| Selects | A name whose media type says `avro`, on any handle, with no format argument |
| Decodes | Columnar, one builder per leaf, no `Scalar` tree; an unselected top-level column is skipped, not decoded, but its row bytes are still read |
| Block codec | `null`, `deflate` (default), `zstandard`; `snappy` in builds with the `parquet` feature |
| Sync marker | Absent (a fresh marker per write) or exactly 16 bytes |
| Record surface refuses | A union wider than `null` plus one branch, a recursive schema, a datatype Avro cannot spell; the `Scalar` functions have no such limits |
| Cached | `open` keeps the inferred wrapper, schema, and dimensions until `close`; writes invalidate |
| Limits | Every Rust reader has a `_with_limits` form over [`Limits`](../text/index.md); Python snake-case keywords, JavaScript camel-case options |

## Use

Read, overwrite, append, and keyed merge follow the [canonical record signatures](../holder/iobase/records.md) once the handle's media type says `avro`.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
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

## Dimensions and opened sessions

`row_size` walks block counts and encoded lengths, jumps each payload positionally, and validates its sync marker without allocating, decompressing, or decoding rows. `column_size` reads only the header schema; both describe the whole container, ignoring selections, filters, and limits.

| form | role |
| --- | --- |
| `avro::Avro` | Handle, options, and the metadata cache that [`IOBase::open`](../holder/iobase/bytes.md) fills and `close` releases |
| `avro::AvroOptions` | The shared record options plus the block codec name and an optional fixed sync marker for byte-reproducible writes |

## Block encoding options

The generic [`RecordOptions`](options.md) exposes both Avro settings without downcasting, and the writer validates the codec name before pulling a row source.

=== "Rust"

    ```rust
    use yggdryl::media::RecordOptions;
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

## Scalar containers

`write_container` writes the schema JSON into the header verbatim, so attributes this implementation does not model, such as Iceberg's `field-id`, survive byte for byte.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

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
    from yggdryl.media import avro

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

The header makes a container self-describing, so `read_container` needs only the bytes and returns schema, metadata, and rows. Rows use the JSON parser's vocabulary: record as mapping, array as sequence, union as the branch value itself, never a wrapper naming the branch.

## Schemas, canonical form, and fingerprints

A `Schema` resolves namespaces, aliases, defaults, and recursive references at parse time; a named-type reference stays a reference, which keeps a recursive schema finite.

=== "Rust"

    ```rust
    use yggdryl::media::avro::Schema;

    let schema = Schema::from_str(
        r#"{"type": "record", "name": "trade", "doc": "one fill", "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "long", "field-id": 2}
        ]}"#,
    )?;

    assert!(!schema.clone().into_canonical_form().contains("doc"));
    assert_eq!(schema.fingerprint().to_le_bytes()[0], 0xF5);
    let text = String::from_utf8(yggdryl::text::json::into_bytes(&schema.into_json())?)?;
    assert!(text.contains("field-id"));
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

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

`fingerprint` hashes the Parsing Canonical Form with CRC-64-AVRO, which strips whitespace, attribute order, docs, unknown attributes, logical annotations, aliases, and defaults. Equality, total ordering, and `stable_hash` use the complete retained JSON document instead.

## Logical types

`date`, `time-millis`/`micros`, `timestamp-millis`/`micros`/`nanos`, `local-timestamp-*`, `uuid` over string and fixed(16), `decimal` over bytes and fixed, and `duration` decode as typed values.

=== "Rust"

    ```rust
    use yggdryl::TimeUnit;
    use yggdryl::holder::Buffer;
    use yggdryl::{Timezone, Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

    let schema = json::from_utf8(
        r#"{"type": "record", "name": "row", "fields": [
            {"name": "day", "type": {"type": "int", "logicalType": "date"}},
            {"name": "at", "type": {"type": "long", "logicalType": "timestamp-micros"}},
            {"name": "price", "type": {"type": "bytes", "logicalType": "decimal",
                                        "precision": 10, "scale": 2}}
        ]}"#,
    )?;
    let row = Scalar::from_record([
        (
            "day",
            Scalar::date32_in(19_782, TimeUnit::Day, Timezone::NAIVE)?,
        ),
        ("at", Scalar::datetime64(
            1_700_000_000_000_000,
            TimeUnit::Microsecond,
            Timezone::UTC,
        )?),
        ("price", Scalar::d128(18_750, 2)),
    ])?;

    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[], &[row.clone()])?;
    assert_eq!(avro::read_container(&handle)?.rows[0], row);
    ```

=== "Python"

    ```python
    from datetime import date, datetime, timezone
    from decimal import Decimal

    from yggdryl.media import avro

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
    assert.equal(decoded.kind, 'd64')
    assert.equal(decoded.unscaled, 18750n)
    assert.equal(decoded.scale, 2)
    ```

A date is `Date32`, a timestamp is `DateTime64` with `UTC`, and a decimal keeps its exact coefficient and scale.

## Reading with a different schema

`read_container_resolved` compiles the specification's resolution matrix into a `Resolution` once per (writer, reader) pair and executes it per row.

=== "Rust"

    ```rust
    use yggdryl::media::avro::Schema;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

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
    from yggdryl.media import avro

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

| writer | reader | rule |
| --- | --- | --- |
| field | field | Match by name or reader alias, in any order |
| `int` | `long`, `float`, `double` | Promotes |
| `long` | `float`, `double` | Promotes |
| `float` | `double` | Promotes |
| `string` | `bytes`, and back | Interchange |
| enum | enum | Symbols map; the reader's default is the fallback |
| union | union | Resolves branch by branch |
| field the reader does not name | absent | Skipped undecoded: length-prefixed values jump by prefix, size-carrying array and map blocks jump as one seek |

## Streaming a large container

`read_blocks` iterates over nothing but `pread`, so any handle works without holding the file in memory. Python and JavaScript copy an already-held byte value into the owning native handle once.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

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
    from yggdryl.media import avro

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

Each `Block` arrives compressed with its row count; `rows` decodes it, `rows_resolved` decodes through a [`Resolution`](#reading-with-a-different-schema), and calling neither skips the block. `read_container` stays the fast case for small self-describing files such as an [Iceberg](iceberg/index.md) manifest.

## Single-object encoding

Each datum frames as `C3 01`, the writer schema's Rabin fingerprint in little-endian order, and the body.

=== "Rust"

    ```rust
    use yggdryl::media::avro::Schema;
    use yggdryl::{Scalar};
    use yggdryl::media::avro;

    let schema = Schema::from_str(r#"{"type":"record","name":"tick","fields":[
        {"name":"price","type":"double"}]}"#)?;
    let value = Scalar::from_record([("price", Scalar::from(187.5))])?;
    let framed = avro::into_single_object_vec(&schema, &value)?;

    assert_eq!(&framed[..2], &[0xC3, 0x01]);
    assert_eq!(avro::from_single_object_slice(&framed, &schema)?, value);
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

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

The fingerprint is how a receiver picks the writer schema out of a store, and the natural key for caching a `Resolution`.

## Codecs and limits

Input bytes bound the container and each decompressed block, depth bounds schema and datum nesting, and the node budget bounds rows and per-datum allocation.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Limits, Scalar};
    use yggdryl::text::json;
    use yggdryl::media::avro;

    let schema = json::from_utf8(r#""long""#)?;
    let mut bytes = Buffer::new();
    avro::write_container(&mut bytes, &schema, &[], &[Scalar::from(7_i64)])?;

    let limits = Limits::new(8, 1_024, 8, 1);
    assert_eq!(
        avro::read_container_with_limits(&bytes, limits)?.rows,
        [Scalar::from(7_i64)]
    );
    ```

=== "Python"

    ```python
    from yggdryl.media import avro

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

| header codec | implementation |
| --- | --- |
| `null`, `deflate`, `zstandard` | The crate's own [`Codec`](../coding/index.md) implementations |
| `snappy` | Raw Snappy followed by a big-endian CRC-32 of the uncompressed block; builds carrying the `parquet` feature |
| `bzip2`, `xz`, any other name | Refused, naming it and listing what this build implements |

## Edges

- `merge_by_names` -> upsert: rows matching the key are updated, misses are inserted.
- `trades.avro.gz` -> refused rather than double-compressed: Avro compresses inside its blocks, like [Parquet](parquet.md) and unlike [IPC](ipc.md).
- A union wider than `null` plus one branch, a recursive schema, or an unspellable datatype -> refused by name on the record surface.
- The same input through the `Scalar` functions -> accepted; they carry no such limit.
- `set_avro_block_codec` or `set_avro_sync_marker` on options for another encoding -> typed record error.
- A sync marker of any length but 16 bytes -> refused; an absent marker generates a fresh one per write.
- An unknown logical annotation, or attributes invalid for its underlying type -> degrades to the underlying type, never an error.
- A decimal wider than 38 digits -> keeps its raw bytes; `duration` keeps its twelve bytes, being a month/day/millisecond triple.
- An illegal resolution -> refused when the plan is built, naming both sides and the field path.
- A union branch the reader cannot accept -> fails only when a datum actually takes it.
- Same fingerprint, different retained JSON -> distinct schema values; the bindings' `equals`, comparison, and hash follow the JSON identity.
- `avro.blocks` in Python or JavaScript -> fused after its first error.
- A low node limit on lazy blocks -> reported at the first row over budget, after the header has opened.
- Opening the mandatory header -> never consumes the row budget; byte and depth bounds still apply.
- A hostile or malformed container -> typed error carrying the byte position at or just after the failure, never an allocation the process dies of.
- A projection -> saves the decode and allocation of skipped columns, never the row read, because Avro interleaves columns per record.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::avro::tests
    cargo test --features "parquet iceberg" -p yggdryl --test interop avro::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/avro
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/avro
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/avro
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_avro.py
    python/.venv/bin/python python/benchmarks/media.py --filter avro
    python/.venv/bin/python scripts/bench_avro_baseline.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/avro.test.js
    YGGDRYL_BENCH_FILTER=records/avro npm run --prefix node bench:media
    ```

## Performance

### Record surface

Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23). The read fixture holds 65,536 rows and four columns; the write fixture holds 4,096 rows, its append and merge base prepared outside the timer.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 26.0 ms | 2.52M rows/s |
| `overwrite_arrow_reader` | 4,096 | 6.10 ms | 671k rows/s |
| `append_arrow_reader` | 4,096 | 12.9 ms | 318k rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 11.4 ms | 358k rows/s |

Opened calls answer from the cache `open` fills; closed calls derive fresh metadata, on the same 65,536-row fixture.

| dimension | fresh | opened |
| --- | ---: | ---: |
| `row_size` | 20.6 us | 83.6 ns |
| `column_size` | 22.3 us | 87.8 ns |
| `read_arrow_field` | 101 us | 72.5 us |

The generic options enum redirects both Avro settings without downcasting or allocation.

| options operation | estimate |
| --- | ---: |
| read the block codec | 9.65 ns |
| set the block codec and a fixed marker | 49.8 ns |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/avro
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/avro
```

### JavaScript raw codec, historical

One 4,580-byte container of 1,000 three-column rows, fixtures outside the loops, from `npm run bench:codec` on Node 24.18.0, an x86-64 Windows release build (AMD Ryzen 5 150). That script no longer exists and has no replacement, so these numbers stand as history.

| operation | ms/op |
| --- | ---: |
| schema parse / canonical form | 0.136 / 0.003 |
| container decode / resolve / encode | 13.075 / 10.549 / 18.667 |
| first compressed block / decode / resolve | 0.054 / 13.833 / 11.979 |
| single-object decode / encode | 0.018 / 0.087 |

The first-block row includes header parsing and the first lazy `next` but not row decompression; the block decode and resolve rows measure that separately.

### Against fastavro and PyIceberg, on identical bytes

`scripts/bench_avro_baseline.py` writes one deterministic ten-thousand-entry [Iceberg](iceberg/index.md) manifest (112,246 bytes, statistics included) from Rust, then times three readers over those exact bytes. One containerized x86_64 Linux run: rustc stable release build, CPython 3.11.15, fastavro 1.12.2, pyiceberg 0.11.1.

```text
fastavro 1.12.2:                    67,719 entries/s best (147.7 ms best of 7)
pyiceberg 0.11.1:                   46,790 entries/s best (213.7 ms best of 7)
yggdryl full (release):            101,937 entries/s best ( 98.1 ms best of 7)
yggdryl plan_stats (release):      203,252 entries/s best ( 49.2 ms best of 7)
yggdryl plan_identity (release):   438,596 entries/s best ( 22.8 ms best of 7)
```

| row | call | keeps |
| --- | --- | --- |
| `full` | `read_manifest` | Every field, the way the other two readers do |
| `plan_stats` | `read_manifest_for_plan(handle, true)`, what a filtered scan runs | The value counts, null counts, and bounds that pruning consults; the rest skipped as bytes |
| `plan_identity` | The unfiltered planning read | File identity, partition tuple, and sizes |

On this manifest the planning path is 2.1x the full decode with statistics kept and 4.4x without. The ratios hold at 1,000 and 100,000 entries: `manifest/decode_full`, `manifest/decode_plan_with_stats`, and `manifest/decode_plan_identity_only` in the `media` bench target.

The script needs `fastavro` and `pyiceberg` installed into `python/.venv` and regenerates its fixture itself.

```bash
python/.venv/bin/python scripts/bench_avro_baseline.py
```

### Codec groups

From the same machine, the five `codec/avro*` groups:

- **Types** (`codec/avro_types`, 10,000 rows each): primitives decode at ~2.9M rows/s and encode at ~3.5M rows/s. Two-string rows decode at ~3.3M rows/s, 18-digit decimals at ~6.7M rows/s, and array of records of maps at ~620K rows/s. The single-object varint floor sits at ~57-65 ns per framed datum.
- **Codec x block size** (`codec/avro_blocks`, 65,536 three-column rows): decode throughput is nearly flat from 1,024 to 65,536 rows per block for every codec. Below ~1,000 rows the per-block header and sync overhead shows. Encoded bytes decode at ~20 MiB/s for snappy, ~12 for deflate, ~9.6 for zstandard, and ~38 for null. Null's bytes are bigger, so compare row rates.
- **Projection** (`codec/avro_projection`, 40 columns, null codec so the skip itself is visible). Reading 3 of 40 columns takes 6.4 ms against 9.4 ms for all 40 over 8,192 rows. The saving is the decode and allocation of the 37 skipped columns, jumped by their length prefixes, never the row read.
- **Resolution** (`codec/avro_resolution`): compiling a five-field plan costs ~533 ns once. Executing it per row beats the direct decode on this shape: 4.12 ms against 4.74 ms for 10,000 rows. The plan skips two writer columns the reader never wanted.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro
```
