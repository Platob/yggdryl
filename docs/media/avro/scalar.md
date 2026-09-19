# Apache Avro scalars

Containers as native values: the header's schema and metadata, rows in the JSON parser's vocabulary, and the single-object framing one datum uses.

## Contract

| Key | Value |
| --- | --- |
| Reads | `read_container` needs only the bytes, `read_blocks` streams them, `read_container_resolved` reads under a different reader schema |
| Writes | `write_container` writes the schema JSON verbatim, so attributes this implementation does not model survive byte for byte |
| Rows | the [`Scalar`](../../types/scalar.md) vocabulary the JSON parser uses: record as mapping, array as sequence, union as the branch value itself |
| One datum | `into_single_object_vec` / `from_single_object_slice`, framed `C3 01` plus the writer schema's Rabin fingerprint |
| Limits | input bytes bound the container and each decompressed block, depth bounds nesting, the node budget bounds rows |
| Bindings | Python `avro.dumps` / `avro.loads` / `avro.blocks`; JavaScript `avro.dumps` / `avro.loads`, the same names camelCased for the single-object pair |

## Use

`write_container` writes the schema JSON into the header verbatim, so attributes this implementation does not model, such as Iceberg's `field-id`, survive byte for byte.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar};
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

    let decoded = avro::read_container(&handle)?;
    assert_eq!(decoded.get("source"), Some("docs"));
    assert_eq!(decoded.rows.len(), 2);
    assert_eq!(
        decoded.rows[0].get_key_str("symbol").and_then(Scalar::as_str),
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

## Reading with a different schema

`read_container_resolved` compiles the specification's resolution matrix into a `Resolution` once per (writer, reader) pair and executes it per row.

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;
    use yggdryl::holder::Buffer;
    use yggdryl::{Scalar};
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
    use yggdryl::json;
    use yggdryl::avro;

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

Each `Block` arrives compressed with its row count; `rows` decodes it, `rows_resolved` decodes through a [`Resolution`](#reading-with-a-different-schema), and calling neither skips the block. `read_container` stays the fast case for small self-describing files such as an [Iceberg](../iceberg/index.md) manifest.

## Single-object encoding

Each datum frames as `C3 01`, the writer schema's Rabin fingerprint in little-endian order, and the body.

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;
    use yggdryl::{Scalar};
    use yggdryl::avro;

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

## Edges

- The `Scalar` functions -> no union, recursion, or datatype limit; those belong to the [record surface](arrow.md).
- An illegal resolution -> refused when the plan is built, naming both sides and the field path.
- A union branch the reader cannot accept -> fails only when a datum actually takes it.
- `avro.blocks` in Python or JavaScript -> fused after its first error.
- A low node limit on lazy blocks -> reported at the first row over budget, after the header has opened.
- Opening the mandatory header -> never consumes the row budget; byte and depth bounds still apply.
- A hostile or malformed container -> typed error carrying the byte position at or just after the failure, never an allocation the process dies of.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib avro::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_avro.py
    python/.venv/bin/python scripts/bench_avro_baseline.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/avro.test.js
    ```
