# Apache Avro batches

The record surface: Avro blocks as streamed Arrow batches, and the datatype mapping a write has to spell.

## Contract

| Key | Value |
| --- | --- |
| Reads | `read_arrow_reader` decodes the container's blocks into batches bounded by `batch_row_size`; `read_arrow_field` answers its schema as a non-null struct root |
| Writes | `overwrite_arrow_reader`, `append_arrow_reader`, keyed `merge_arrow_reader` under the [canonical signatures](../../holder/iobase/records.md) |
| Pushdown | a projection saves the decode and allocation of skipped columns, never the row read: Avro interleaves columns per record |
| Refuses | a union wider than `null` plus one branch, a recursive schema, or a datatype Avro cannot spell, by name |
| Blocks | `set_avro_block_codec` and `set_avro_sync_marker` on [`RecordOptions`](../options.md) shape what a write emits |

## Use

The handle's media type selects Avro, so no call names a format. The three intents are the ones [every encoding has](../../holder/iobase/records.md#write-intents); what is Avro's own is the shape of the result - an object container whose header carries the schema every block was written under.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, StructureType, Url};

    let field = DataType::from(StructureType::from_fields([
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

Avro's `string` is UTF-8, so every [string](../../types/text.md) on text storage - every UTF-8 and US-ASCII leaf, any shape, bounded or not - writes as `string`, a fixed width trimmed of its padding; a windows-1252 leaf holds bytes that are not UTF-8 and is refused by name. A fixed byte layout is Avro's `fixed`, every other one is `bytes`, and a maximum is dropped on write because Avro has none: the values were held to it when they entered.

| `DataType` | Avro | Reads back as |
| --- | --- | --- |
| `utf8`, `sized_utf8(n)`, `large_utf8`, `utf8_view`, `ascii`, `fixed_ascii(n)`, `fixed_utf8(n)` | `string` | `utf8` |
| `cp1252`, every windows-1252 leaf | refused: `expected a datatype Avro can spell` | |
| `country`, `currency`, `mic`, `cfi`, `isin` | `string` | `utf8` |
| `binary`, `sized_binary(n)`, `large_binary`, `binary_view`, `large_binary_view` | `bytes` | `binary` |
| `fixed_binary(n)` | `fixed` of size `n` | `fixed_binary(n)` |
| `uuid` | `string` with `logicalType: uuid` | `uuid` |

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{BinaryArray, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, StructureType, Url};

    let row = DataType::from(StructureType::from_fields([
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

    let legacy = DataType::from(StructureType::from_fields([
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

    from yggdryl import IOBase, types

    row = types.struct("row", [
        types.fixed_ascii("code", 4),
        types.fixed_size_binary("key", 2),
        types.bytes("blob", max=8),
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
    legacy.field = types.struct("row", [types.cp1252("note")], nullable=False)
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
- A union wider than `null` plus one branch, a recursive schema, or an unspellable datatype -> refused by name on the record surface.
- A string in a charset other than UTF-8 or US-ASCII -> refused by name; `fixed_ascii(n)` writes `string` with its padding trimmed, `binary(n)` writes `bytes` with the maximum dropped.
- The same input through the [`Scalar` functions](scalar.md) -> accepted; they carry no such limit.
- A projection -> saves the decode and allocation of skipped columns, never the row read, because Avro interleaves columns per record.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test interop avro::
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/avro
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/avro
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
