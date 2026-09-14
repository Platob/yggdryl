# Arrow IPC batches

Batch readers in and out of an IPC stream, with the projection and schema rules a stream adds.

## Contract

| Key | Value |
| --- | --- |
| Reads | `ipc::read_batch_reader`, `read_arrow_reader` - an [`arrow::BatchReader`](../../arrow/readers.md) whose schema is known before the first batch |
| Writes | `overwrite_arrow_reader`, `append_arrow_reader`, keyed `merge_arrow_reader`; every one publishes through `ipc::overwrite_arrow_reader` |
| Lazy | a batch is encoded as the writer pulls it and decoded as the reader is stepped; only the current one is alive |
| Pushdown | `field`, a non-null struct root naming a subset, drops columns at decode; it never casts and never skips the message body |
| Schema | self-describing, so a reader that declares nothing recovers it; only the root name is chosen on this side |
| Bindings | Python `pyarrow.RecordBatchReader` over the Arrow C Stream; JavaScript Arrow JS over the copied [IPC boundary](../../extensions/javascript.md) |

## Use

The three intents share one reader-shaped input. Append retains stored rows; keyed merge updates matching `id` values and inserts misses. Nothing is materialized on either side: each batch is encoded as the writer pulls it, and a read yields them back one at a time, block boundaries included.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
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

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![1, 2], vec![Some("XNAS"), Some("XNYS")])?],
        ),
        &options,
    )?;
    handle.append_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![3], vec![Some("XLON")])?],
        ),
        &options,
    )?;
    handle.merge_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![2, 4], vec![Some("XPAR"), None])?],
        ),
        &options.clone().with_merge_by(["id"])?,
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

    # The name says Arrow IPC, so no call names an encoding.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(batch([1, 2], ["XNAS", "XNYS"]))
    handle.append_arrow_batch(batch([3], ["XLON"]))

    merging = handle.record_options()
    merging.merge_by = ["id"]
    handle.merge_arrow_batch(batch([2, 4], ["XPAR", None]), options=merging)

    assert handle.read_arrow_field().name == "row"
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

    // The name says Arrow IPC, so no call names an encoding.
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.arrows'))
    handle.overwriteArrowTable(rows([1, 2], ['XNAS', 'XNYS']))
    handle.appendArrowTable(rows([3], ['XLON']))
    handle.mergeArrowTable(
      rows([2, 4], ['XPAR', null]),
      handle.recordOptions().withMergeBy(['id']),
    )

    assert.equal(handle.readArrowField().name, 'row')
    assert.equal(handle.readArrowReader().intoTable().numRows, 4)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Column pushdown

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    let stored = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
        DataType::utf8().required_field("venue"),
    ])?
    .required_field("row");
    let arrow_schema = stored.into_arrow_schema()?;

    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            Arc::new(StringArray::from(vec!["XNAS", "XNAS"])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = IpcOptions::new();
    ipc::overwrite_arrow_reader(&mut handle, arrow::batch_reader(arrow_schema, [batch]), &options)?;

    // One of the three columns, named by a root Field of its own.
    let wanted = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let projected = ipc::read_batch_reader(&handle, Some(&wanted), &options)?;
    assert_eq!(projected.schema().fields().len(), 1);
    let batches = projected.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(batches[0].num_columns(), 1);

    // The stream itself is unchanged: it still carries all three.
    assert_eq!(ipc::read_field(&handle, &options)?.field_len(), 3);
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
        pa.field("venue", pa.string(), nullable=False),
    ])
    batch = pa.record_batch(
        {"id": [1, 2], "symbol": ["AAPL", "MSFT"], "venue": ["XNAS", "XNAS"]},
        schema=stored,
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(batch)

    # One of the three columns, declared through the centralized options field.
    options = handle.record_options()
    options.field = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    projected = handle.read_arrow_reader(options=options)
    assert projected.schema.names == ["id"]
    assert projected.read_all().num_columns == 1

    # The stream itself is unchanged: it still carries all three.
    assert len(handle.read_arrow_field().dtype) == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, IOBase, MimeType, fields } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
        venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
      }),
    )

    // One of the three columns, declared as this read's schema.
    const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    const projected = handle.readArrowReader(handle.recordOptions().withField(wanted))
    assert.deepEqual([...projected.field.dtype].map((child) => child.name), ['id'])
    assert.equal(projected.intoTable().numCols, 1)

    // The stream itself is unchanged: it still carries all three.
    assert.equal(handle.readArrowField().dtype.length, 3)
    ```

The `field` argument is a column pushdown and nothing else: skipped columns are never decoded or allocated. An IPC batch body is one message and is still read whole; [Parquet](../parquet/index.md) is where a projection also removes reading.

## The stream carries its schema

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::media::DEFAULT_ROOT_NAME;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::Ipc;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from(vec![7]))],
    )?;

    let mut writer = Ipc::new(Buffer::new()).with_field(schema.clone());
    let options = writer.record_options()?;
    writer.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
    let bytes = writer.handle().as_slice().to_vec();

    // A reader that declares nothing recovers the schema from the bytes.
    let reader = Ipc::new(Buffer::from_bytes(bytes.clone()));
    let options = reader.record_options()?;
    assert_eq!(reader.read_arrow_field(&options)?, schema);
    assert_eq!(reader.read_arrow_field(&options)?.name(), DEFAULT_ROOT_NAME);

    // Arrow names columns, not the record; the root name is chosen on this side.
    let named = Ipc::new(Buffer::from_bytes(bytes)).with_name("trade");
    let options = named.record_options()?;
    let named_field = named.read_arrow_field(&options)?;
    assert_eq!(named_field.name(), "trade");
    assert_eq!(named_field.get_field_by_path("id"), schema.get_field_by_path("id"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(pa.record_batch({"id": [7]}, schema=schema))

    # A reader that declares nothing recovers the schema from the bytes.
    assert handle.read_arrow_field().name == "row"

    # Arrow names columns, not the record; the root name is chosen on this side.
    named = handle.record_options()
    named.name = "trade"
    assert handle.read_arrow_field(options=named).name == "trade"
    assert [child.name for child in handle.read_arrow_field().dtype] == ["id"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowTable(
      new arrow.Table({ id: arrow.vectorFromArray([7n], new arrow.Int64()) }),
    )

    // A reader that declares nothing recovers the schema from the bytes.
    assert.equal(handle.readArrowField().name, 'row')

    // Arrow names columns, not the record; the root name is chosen on this side.
    const named = handle.recordOptions().withName('trade')
    assert.equal(handle.readArrowField(named).name, 'trade')
    assert.deepEqual([...handle.readArrowField().dtype].map((child) => child.name), ['id'])
    ```

Arrow names the columns and not the record, so the root name is the one thing inference cannot recover.

## Edges

- `merge_by` on an overwrite or append -> the key never selects merge; intent stays with the method name.
- `append_*` or keyed `merge_*` -> published through `ipc::overwrite_arrow_reader`, the one complete-stream encoder.
- `field` naming every stored column, or one the stream lacks -> reads everything; a projection only drops columns.
- projected read -> the returned reader reports the projected schema; Arrow's own `StreamReader` would report the whole stream's.
- reading -> batches arrive one at a time and only the current one is alive.
- root name -> Arrow names the columns and not the record, so it is the one thing inference cannot recover.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::ipc::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/ipc
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_ipc.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    ```
