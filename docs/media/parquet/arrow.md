# Apache Parquet batches

Batch readers in and out of a Parquet file, and the projection that removes reading rather than only decoding.

## Contract

| Key | Value |
| --- | --- |
| Reads | `read_arrow_reader` returns an [`arrow::BatchReader`](../../arrow/readers.md), `read_arrow_field` the canonical non-null struct root [`Field`](../../types/field.md) |
| Writes | `overwrite_arrow_reader`, `append_arrow_reader`, keyed `merge_arrow_reader` under the [canonical signatures](../../holder/iobase/records.md) |
| Pushdown | the read `field` becomes a `ProjectionMask` over root columns; excluded chunks are never located, decompressed, or decoded |
| Row groups | `max_row_group_size`, default 1,048,576, bounds what one group holds; `batch_row_size` bounds what one batch carries |
| Bindings | Python exchanges `pyarrow.RecordBatchReader` over the Arrow C Stream; JavaScript exchanges Arrow JS values over copied IPC, one batch per stream; neither builds a table unasked |

## Use

The handle's media type selects Parquet, so no call names a format. The three intents are the ones [every encoding has](../../holder/iobase/records.md#write-intents) - append rewrites the file around the stored rows, keyed merge upserts - and what is Parquet's own is the result: a whole file any reader opens.

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
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), Some("MSFT")])),
        ],
    )?;

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.parquet")?.media_type());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    assert_eq!(handle.read_arrow_reader(&options)?.count(), 1);
    assert_eq!(handle.read_arrow_field(&options)?.field_len(), 2);
    // A whole Parquet file, magic and footer included.
    assert_eq!(&handle.read_all_bytes()?[..4], b"PAR1");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pyarrow.parquet as pq

    from yggdryl import IOBase

    path = pathlib.Path(tempfile.mkdtemp()) / "trades.parquet"
    handle = IOBase(path)
    handle.overwrite_arrow_table(pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}))
    handle.append_arrow_table(pa.table({"id": [3], "symbol": ["GOOG"]}))

    assert handle.read_arrow_reader().read_all().num_rows == 3
    # A whole Parquet file, so any other reader opens it.
    assert pq.read_table(path).num_rows == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const rows = (ids, symbols) => new arrow.Table({
      id: arrow.vectorFromArray(ids.map(BigInt), new arrow.Int64()),
      symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
    })

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteArrowTable(rows([1, 2], ['AAPL', 'MSFT']))
    handle.appendArrowTable(rows([3], ['GOOG']))

    assert.equal(handle.readArrowReader().intoTable().numRows, 3)
    // A whole Parquet file, magic and footer included.
    assert.deepEqual([...handle.readBytes().subarray(0, 4)], [...Buffer.from('PAR1')])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Column pushdown

A non-null struct root naming a subset of the stored columns reads only those chunks, in stored order with stored types. Unlike an [Arrow IPC](../ipc/index.md) batch, one contiguous message, a Parquet column chunk is separately addressable.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Float64Array, Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType, StructureType};

    let stored = DataType::from(StructureType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::utf8().required_field("venue"),
    ])?)
    .required_field("row");
    let arrow_schema = stored.into_arrow_schema()?;

    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            Arc::new(Float64Array::from(vec![1.5, 2.5])),
            Arc::new(StringArray::from(vec!["XNAS", "XNAS"])),
        ],
    )?;

    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;

    // Two of the four columns, named by a root Field of its own.
    let wanted = DataType::from(StructureType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])?)
    .required_field("row");

    let projected = media.read_arrow_reader(&options.clone().with_field(wanted))?;
    assert_eq!(projected.schema().fields().len(), 2);
    let read = projected.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(read[0].num_columns(), 2);

    // The file is unchanged: it still stores all four.
    assert_eq!(media.read_arrow_schema()?.fields().len(), 4);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    stored = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("price", pa.float64(), nullable=False),
        pa.field("venue", pa.string(), nullable=False),
    ])
    rows = 4_096
    batch = pa.record_batch(
        {
            "id": list(range(rows)),
            "symbol": ["AAPL"] * rows,
            "price": [1.5] * rows,
            "venue": ["XNAS"] * rows,
        },
        schema=stored,
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
    handle.overwrite_arrow_batch(batch)

    # Two of the four columns, declared through the centralized options field.
    options = handle.record_options()
    options.field = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("price", pa.float64(), nullable=False),
    ])
    projected = handle.read_arrow_reader(options=options).read_all()
    assert projected.column_names == ["id", "price"]

    # Less is read, and the bytes say so rather than the clock.
    whole = handle.read_arrow_reader().read_all()
    assert projected.nbytes * 2 <= whole.nbytes

    # The file is unchanged: it still stores all four.
    assert len(handle.read_arrow_field().dtype) == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, IOBase, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
        price: arrow.vectorFromArray([1.5, 2.5], new arrow.Float64()),
        venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
      }),
    )

    // Two of the four columns, declared as this read's schema.
    const wanted = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('price: float64')],
      { nullable: false },
    )

    const options = handle.recordOptions().withField(wanted)
    const projected = handle.readArrowReader(options).intoTable()
    assert.equal(projected.numCols, 2)
    assert.deepEqual(projected.schema.fields.map((child) => child.name), ['id', 'price'])

    // The file is unchanged: it still stores all four.
    assert.equal(handle.readArrowField().dtype.length, 4)

    fs.rmSync(root, { recursive: true, force: true })
    ```

| root field | mask |
| --- | --- |
| a subset of the stored columns | only those chunks; a caller wanting another shape casts afterwards |
| a nested column | its whole subtree; the mask is built from roots, not leaves |
| every stored column, or one the file lacks | everything; a mask drops columns, never invents them |

## Edges

- read `field` naming every stored column, or one the file lacks -> reads everything; a projection only drops columns.
- projected read -> the reader reports the projected schema, and the skipped chunks are never touched.
- a pulled batch not matching the reader's field -> error naming the batch index.
- keyed `merge_arrow_reader` -> upsert: rows matching `merge_by` are updated, misses are inserted.
- `merge_by` on an overwrite or append -> refused; intent stays with the method name.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib parquet::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/parquet
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_parquet.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    ```
