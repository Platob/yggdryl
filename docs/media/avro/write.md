# Apache Avro writes

Rows into Avro bytes: containers and single datums as native scalars first, then Arrow batches, and the datatype mapping a write has to spell.

## Contract

| Item | Behaviour |
| --- | --- |
| Native rows | `write_container` takes the schema JSON, the header metadata, and the rows; Python `avro.dumps`, JavaScript `avro.dumps` |
| Schema | written into the header verbatim, so attributes this implementation does not model - Iceberg's `field-id` among them - survive byte for byte |
| Blocks | one `write_container` call writes every row as one block compressed with raw `deflate`; the record surface takes its codec and marker from [Blocks](blocks.md) |
| Sync marker | `write_container` derives it from the schema and the encoded rows, so the same rows produce the same bytes; the record surface draws a fresh one unless [`RecordOptions`](blocks.md) fixes it |
| One datum | `into_single_object_vec` / `from_single_object_slice`, framed `C3 01` plus the writer schema's Rabin fingerprint |
| Batches | `overwrite_arrow_reader`, `append_arrow_reader`, keyed `merge_arrow_reader` and their record siblings under the [canonical signatures](../../holder/iobase/records.md#write-intents); the media type selects Avro, so no call names a format |
| Strings | every UTF-8 and US-ASCII leaf writes as `string`, a fixed byte layout as `fixed`, every other one as `bytes`; a windows-1252 leaf is refused by name |
| Record surface refuses | a union wider than `null` plus one branch, a recursive schema, a datatype Avro cannot spell; the `Scalar` functions have no such limits |
| Options | the shared [record options](../options.md), plus the block codec and the sync marker on [Blocks](blocks.md) |
| Bindings | Python `avro.dumps` / `avro.dumps_single`; JavaScript `avro.dumps` / `avro.dumpsSingle` |

## Use

The schema JSON is written into the header as it was given, so an attribute this implementation does not model survives the round trip; the metadata pairs sit beside it, and every row of the call becomes one deflate block.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::IOBase;
    use yggdryl::json;
    use yggdryl::avro;

    let schema = json::from_utf8(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"symbol","type":"string","field-id":1},
            {"name":"quantity","type":"long","field-id":2}]}
        "#,
    )?;
    let rows = [
        json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?,
        json::from_utf8(r#"{"symbol":"MSFT","quantity":25}"#)?,
    ];
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &schema, &[("source", "docs")], &rows)?;

    let bytes = handle.read_all_bytes()?;
    assert_eq!(&bytes[..4], b"Obj\x01");
    // The header keeps the schema JSON verbatim: `field-id` is not modeled
    // here, and survives anyway.
    assert!(String::from_utf8_lossy(&bytes).contains("field-id"));

    let decoded = avro::read_container(&handle)?;
    assert_eq!(decoded.get("source"), Some("docs"));
    assert_eq!(decoded.rows.len(), 2);
    ```

=== "Python"

    ```python
    from yggdryl import avro

    schema = {
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "symbol", "type": "string", "field-id": 1},
            {"name": "quantity", "type": "long", "field-id": 2},
        ],
    }
    encoded = avro.dumps(
        [{"symbol": "AAPL", "quantity": 100}, {"symbol": "MSFT", "quantity": 25}],
        schema,
        metadata={"source": "docs"},
    )

    assert encoded[:4] == b"Obj\x01"
    # The header keeps the schema JSON verbatim: `field-id` is not modeled
    # here, and survives anyway.
    assert b"field-id" in encoded

    decoded = avro.loads(encoded)
    assert decoded.metadata == {"source": "docs"}
    assert len(decoded.rows) == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const schema = {
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'symbol', type: 'string', 'field-id': 1 },
        { name: 'quantity', type: 'long', 'field-id': 2 },
      ],
    }
    const encoded = avro.dumps(
      [{ symbol: 'AAPL', quantity: 100 }, { symbol: 'MSFT', quantity: 25 }],
      schema,
      { source: 'docs' },
    )

    assert.deepEqual(encoded.subarray(0, 4), Buffer.from('Obj\x01'))
    // The header keeps the schema JSON verbatim: `field-id` is not modeled
    // here, and survives anyway.
    assert.ok(encoded.includes(Buffer.from('field-id')))

    const decoded = avro.loads(encoded)
    assert.deepEqual(decoded.metadata, { source: 'docs' })
    assert.equal(decoded.rows.length, 2)
    ```

## Single-object encoding

Each datum frames as `C3 01`, the writer schema's Rabin fingerprint in little-endian order, and the body.

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;
    use yggdryl::Scalar;
    use yggdryl::avro;

    let schema = Schema::from_str(r#"{"type":"record","name":"tick","fields":[
        {"name":"price","type":"double"}]}"#)?;
    let value = Scalar::from_struct([("price", Scalar::from(187.5))])?;
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

The fingerprint is how a receiver picks the writer schema out of a store, and the natural key for caching a [`Resolution`](read.md#reading-with-a-different-schema). Its canonical form and the hash over it are on [Schemas](schemas.md).

## Rows as Arrow batches

The handle's media type selects Avro, so no call names a format. The three intents are the ones [every encoding has](../../holder/iobase/records.md#write-intents); what is Avro's own is the shape of the result - an object container whose header carries the schema every block was written under.

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

    assert_eq!(handle.read_arrow_reader(&options)?.count(), 1);
    assert_eq!(handle.read_arrow_field(&options)?.field_len(), 2);
    // An object container: the header carries the schema the blocks hold.
    assert_eq!(&handle.read_all_bytes()?[..4], b"Obj\x01");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.avro")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2], "venue": ["XNAS", "XNYS"]}))
    handle.append_arrow_table(pa.table({"id": [3], "venue": ["XLON"]}))

    assert handle.read_arrow_reader().read_all().num_rows == 3
    # An object container: the header carries the schema the blocks hold.
    assert handle.read_bytes()[:4] == b"Obj\x01"
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

    assert.equal(handle.readArrowReader().intoTable().numRows, 3)
    // An object container: the header carries the schema the blocks hold.
    assert.deepEqual([...handle.readBytes().subarray(0, 4)], [...Buffer.from('Obj\x01')])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Strings and bytes on the wire

Avro's `string` is UTF-8, so every [string](../../types/index.md) on text storage - every UTF-8 and US-ASCII leaf, any shape, bounded or not - writes as `string`, a fixed width trimmed of its padding; a windows-1252 leaf holds bytes that are not UTF-8 and is refused by name. A fixed byte layout is Avro's `fixed`, every other one is `bytes`, and a maximum is dropped on write because Avro has none: the values were held to it when they entered.

| `DataType` | Avro | Reads back as |
| --- | --- | --- |
| `utf8`, `sized_utf8(n)`, `large_utf8`, `utf8_view`, `ascii`, `fixed_ascii(n)`, `fixed_utf8(n)` | `string` | `utf8` |
| `cp1252`, every windows-1252 leaf | refused: `expected a datatype Avro can spell` | |
| `country`, `currency`, `mic`, `cfi`, `isin` | `string` | `utf8` |
| `binary`, `sized_binary(n)`, `large_binary`, `binary_view`, `large_binary_view` | `bytes` | `binary` |
| `fixed_binary(n)` | `fixed` of size `n` | `fixed_binary(n)` |
| `uuid` | `string` with `logicalType: uuid` | `uuid` |
| `variant` | a `record` of `metadata` and `value`, both `bytes`, with `logicalType: variant` | `variant` |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{BinaryArray, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, StructType, Url};

    let row = DataType::from(StructType::from_fields([
        DataType::fixed_ascii(4)?.nullable_field("code"),
        DataType::fixed_binary(2)?.nullable_field("key"),
        DataType::from_str("binary(8)")?.nullable_field("blob"),
    ])?)
    .required_field("row");
    let plain = RecordBatch::try_from_iter([
        ("code", Arc::new(StringArray::from(vec!["AB"])) as _),
        ("key", Arc::new(BinaryArray::from(vec![&[0_u8, 1][..]])) as _),
        ("blob", Arc::new(BinaryArray::from(vec![&b"xyz"[..]])) as _),
    ])?;

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///codes.avro")?.media_type());
    let options = handle.record_options()?;
    // The declared field casts the plain batch once on the way in.
    handle.overwrite_arrow_batch(plain, &options.clone().with_field(row))?;

    let stored = handle.read_arrow_field(&options)?;
    let spelled: Vec<String> = stored.fields().iter().map(|child| child.dtype().to_string()).collect();
    assert_eq!(spelled, ["utf8", "fixed_binary(2)", "binary"]);

    let legacy = DataType::from(StructType::from_fields([
        DataType::cp1252().nullable_field("note"),
    ])?)
    .required_field("row");
    let note = RecordBatch::try_from_iter([("note", Arc::new(StringArray::from(vec!["hi"])) as _)])?;
    let refused = Buffer::new()
        .with_media_type(Url::from_str("file:///notes.avro")?.media_type())
        .overwrite_arrow_batch(note, &options.with_field(legacy))
        .unwrap_err()
        .to_string();
    assert!(refused.contains("expected a datatype Avro can spell, got cp1252"), "{refused}");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    import yggdryl

    from yggdryl import IOBase

    row = yggdryl.struct("row", [
        yggdryl.fixed_ascii("code", 4),
        yggdryl.fixed_size_binary("key", 2),
        yggdryl.bytes("blob", max=8),
    ], nullable=False)

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "codes.avro")
    options = handle.record_options()
    options.field = row
    # The declared field casts the plain batch once on the way in.
    handle.overwrite_arrow_batch(
        pa.record_batch({"code": ["AB"], "key": [b"\x00\x01"], "blob": [b"xyz"]}),
        options=options,
    )

    stored = handle.read_arrow_field()
    assert [str(child.dtype) for child in stored.dtype] == ["utf8", "fixed_binary(2)", "binary"]
    assert list(handle.read_records()) == [{"code": "AB", "key": b"\x00\x01", "blob": b"xyz"}]

    legacy = handle.record_options()
    legacy.field = yggdryl.struct("row", [yggdryl.cp1252("note")], nullable=False)
    try:
        IOBase(pathlib.Path(tempfile.mkdtemp()) / "notes.avro").overwrite_arrow_batch(
            pa.record_batch({"note": ["hi"]}), options=legacy
        )
    except ValueError as error:
        assert "expected a datatype Avro can spell, got cp1252" in str(error), error
    else:
        raise AssertionError("a windows-1252 string is not Avro's string")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, fields } = require('yggdryl')

    const row = fields.struct(
      'row',
      [
        fields.fixedAscii('code', 4),
        fields.fixedSizeBinary('key', 2),
        fields.bytes('blob', { max: 8 }),
      ],
      { nullable: false },
    )
    const plain = new arrow.Table({
      code: arrow.vectorFromArray(['AB'], new arrow.Utf8()),
      key: arrow.vectorFromArray([Uint8Array.from([0, 1])], new arrow.Binary()),
      blob: arrow.vectorFromArray([Buffer.from('xyz')], new arrow.Binary()),
    })

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'codes.avro'))
    // The declared field casts the plain table once on the way in.
    handle.overwriteArrowReader(BatchReader.from(plain), handle.recordOptions().withField(row))

    const stored = handle.readArrowField()
    assert.deepEqual(
      [0, 1, 2].map((index) => String(stored.dtype.getFieldAt(index).dtype)),
      ['utf8', 'fixed_binary(2)', 'binary'],
    )
    assert.deepEqual([...handle.readRecords()].map((record) => record.code), ['AB'])

    const legacy = fields.struct(
      'row',
      [fields.cp1252('note')],
      { nullable: false },
    )
    assert.throws(
      () =>
        new IOBase(path.join(root, 'notes.avro')).overwriteArrowReader(
          BatchReader.from(new arrow.Table({ note: arrow.vectorFromArray(['hi'], new arrow.Utf8()) })),
          handle.recordOptions().withField(legacy),
        ),
      /expected a datatype Avro can spell, got cp1252/,
    )
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Edges

- `merge_by` -> upsert: rows matching the key are updated, misses are inserted.
- a union wider than `null` plus one branch, a recursive schema, or an unspellable datatype -> refused by name on the record surface.
- a string in a charset other than UTF-8 or US-ASCII -> refused by name; `fixed_ascii(n)` writes `string` with its padding trimmed, `binary(n)` writes `bytes` with the maximum dropped.
- the same input through the [`Scalar` functions](#use) -> accepted; they carry no such limit.
- a row that does not fit the writer schema -> refused as it is encoded, rather than written.
- every row of one `write_container` call -> one `deflate` block; the record surface's codec, block size, and marker are [Blocks](blocks.md).
- a sync marker of any length but 16 bytes -> refused ([Blocks](blocks.md)).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test media avro::
    cargo test --features "parquet iceberg" -p yggdryl --test interop avro::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/avro
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_avro.py
    python/.venv/bin/python python/benchmarks/media.py --filter avro
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/avro.test.js
    YGGDRYL_BENCH_FILTER=records/avro npm run --prefix node bench:media
    ```
