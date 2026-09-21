# Apache Avro reads

Rows out of Avro bytes: containers as native scalars first, then Arrow batches, with writer/reader resolution and block streaming in between.

## Contract

| Item | Behaviour |
| --- | --- |
| Native rows | `read_container` needs only the bytes, because the header carries the writer schema; Python `avro.loads`, JavaScript `avro.loads` |
| Row shape | the [`Scalar`](../../types/scalar.md) vocabulary the JSON parser uses: record as mapping, array as sequence, union as the branch value itself, never a wrapper naming the branch |
| Resolved | `read_container_resolved` compiles the specification's resolution matrix into a `Resolution` once per (writer, reader) pair and executes it per row; a binding `reader_schema` option compiles and reuses it internally |
| Lazy | `read_blocks` streams a container over nothing but `pread`; each block arrives compressed with its row count and decodes only when asked |
| Batches | `read_arrow_reader` decodes the container's blocks into batches bounded by `batch_row_size`; `read_arrow_field` answers its schema as a non-null struct root [`Field`](../../types/field.md) |
| Pushdown | a projection saves the decode and allocation of skipped columns, never the row read: Avro interleaves columns per record |
| One datum | a framed single-object datum reads with `from_single_object_slice` / `loads_single`, beside the encoder on [Write](write.md#single-object-encoding) |
| Limits | every Rust reader has a `_with_limits` form over [`Limits`](../structured.md); Python snake-case keywords, JavaScript camel-case options: [Blocks](blocks.md#codecs-and-limits) |
| Bindings | Python `avro.loads`, `avro.blocks`, `avro.loads_single`; JavaScript `avro.loads`, `avro.blocks`, `avro.loadsSingle` |

## Use

An object container is self-describing: the header names the writer schema and carries the user metadata, so a read needs nothing but the bytes and answers with all three.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::Scalar;
    use yggdryl::json;
    use yggdryl::avro;

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

    // Schema, metadata, and rows, from the bytes alone.
    let decoded = avro::read_container(&handle)?;
    assert_eq!(decoded.schema.kind(), "record");
    assert_eq!(decoded.get("source"), Some("docs"));
    assert_eq!(decoded.rows.len(), 2);
    assert_eq!(
        decoded.rows[0].get_key_str("symbol").and_then(Scalar::as_str),
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

    # Schema, metadata, and rows, from the bytes alone.
    decoded = avro.loads(encoded)
    assert decoded.schema.kind == "record"
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

    // Schema, metadata, and rows, from the bytes alone.
    const decoded = avro.loads(encoded)
    assert.equal(decoded.schema.kind, 'record')
    assert.deepEqual(decoded.metadata, { source: 'docs' })
    assert.deepEqual(decoded.rows[0], { quantity: 100, symbol: 'AAPL' })
    ```

## Reading with a different schema

`read_container_resolved` compiles the specification's resolution matrix into a `Resolution` once per (writer, reader) pair and executes it per row.

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;
    use yggdryl::holder::Buffer;
    use yggdryl::Scalar;
    use yggdryl::json;
    use yggdryl::avro;

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
    use yggdryl::Scalar;
    use yggdryl::json;
    use yggdryl::avro;

    let schema = json::from_utf8(r#"{"type":"record","name":"row","fields":[
        {"name":"id","type":"long"}]}"#)?;
    let rows: Vec<Scalar> = (0..3)
        .map(|id| Scalar::from_struct([("id", Scalar::from(id))]))
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

Each `Block` arrives compressed with its row count; `rows` decodes it, `rows_resolved` decodes through a [`Resolution`](#reading-with-a-different-schema), and calling neither skips the block. `read_container` stays the fast case for small self-describing files such as an [Iceberg](../iceberg/index.md) manifest.

## Rows as Arrow batches

The same rows, without the value model in the middle: the handle's media type selects Avro, `read_arrow_reader` streams the container's blocks as batches, and `read_arrow_field` names the root it read them under.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, StructType, Url};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )?;

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.avro")?.media_type());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    // A stream of batches, and the non-null struct root the header spells.
    assert_eq!(handle.read_arrow_reader(&options)?.count(), 1);
    assert_eq!(handle.read_arrow_field(&options)?.field_len(), 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.avro")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2], "venue": ["XNAS", "XNYS"]}))

    # A pyarrow.RecordBatchReader: collecting it is the caller's choice.
    assert handle.read_arrow_reader().read_all().num_rows == 2
    assert len(handle.read_arrow_field().dtype) == 2

    # The same rows as native values, one row at a time.
    assert [row["venue"] for row in handle.read_records()] == ["XNAS", "XNYS"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.avro'))
    handle.overwriteArrowTable(new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
    }))

    // One Arrow JS batch per stream; intoTable is the caller's choice.
    assert.equal(handle.readArrowReader().intoTable().numRows, 2)
    assert.equal(handle.readArrowField().dtype.length, 2)

    // The same rows as native values, one row at a time.
    assert.deepEqual([...handle.readRecords()].map((row) => row.venue), ['XNAS', 'XNYS'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

A read `field` naming fewer columns than the container holds is a projection: the skipped columns are jumped by their length prefixes rather than decoded and allocated, but every row is still read, because Avro interleaves its columns per record.

## Edges

- the `Scalar` functions -> no union, recursion, or datatype limit; those bound the record surface only ([Write](write.md#contract)).
- an illegal resolution -> refused when the plan is built, naming both sides and the field path.
- a union branch the reader cannot accept -> fails only when a datum actually takes it.
- `avro.blocks` in Python or JavaScript -> fused after its first error.
- a low node limit on lazy blocks -> reported at the first row over budget, after the header has opened.
- opening the mandatory header -> never consumes the row budget; byte and depth bounds still apply ([Blocks](blocks.md#codecs-and-limits)).
- a hostile or malformed container -> typed error carrying the byte position at or just after the failure, never an allocation the process dies of.
- an empty handle on the record surface -> zero batches under the declared root, not a missing-header error.
- a projection -> saves the decode and allocation of skipped columns, never the row read.

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test avro -- mod_::internal
    cargo test --features "parquet iceberg" -p yggdryl --test avro -- arrow::avro::logical batch::avro::limits batch::avro::records container::avro::containers container::avro::hardening container::avro::snappy container::avro::snapshots container::avro::streaming datum::avro::hardening datum::avro::matrix mod_::fuzz_lite resolve::avro::resolution schema::avro::schemas single::avro::single_object single::avro::snapshots
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/avro
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_avro.py
    python/.venv/bin/python scripts/bench_avro_baseline.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/avro.test.js
    YGGDRYL_BENCH_FILTER=records/avro npm run --prefix node bench:media
    ```
