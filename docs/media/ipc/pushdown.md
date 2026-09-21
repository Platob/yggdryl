# Arrow IPC pushdown

A root `Field` naming a subset of the stored columns projects the read, and the stream still answers with the schema it carries.

## Contract

| Key | Value |
| --- | --- |
| Pushdown | `options.field`, a non-null struct root naming a subset, drops columns at decode |
| Casting | never; stored order and stored types are kept |
| Reading | an IPC batch body is one message and is still read whole, so only decode and allocation are skipped |
| Widening | a `field` naming every stored column, or one the stream lacks, reads everything |
| Schema | self-describing, so a reader that declares nothing recovers it; only the root name is chosen on this side |
| Reported | a projected read reports the projected schema, not the stream's |

## Use

One of three stored columns, named by a root `Field` of its own. Skipped columns are never decoded or allocated, and the stream itself is untouched.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::holder::Buffer;
    use yggdryl::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType, StructType};

    let stored = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
        DataType::utf8().required_field("venue"),
    ])?)
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
    let wanted = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");

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
    use yggdryl::ipc::Ipc;
    use yggdryl::{DataType, StructType};

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");
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

- `field` naming every stored column, or one the stream lacks -> reads everything; a projection only drops columns.
- projected read -> the returned reader reports the projected schema; Arrow's own `StreamReader` would report the whole stream's.
- projection -> drops decode and allocation, never the message body; [Records](../../holder/iobase/records.md) states what each encoding skips.
- root name -> Arrow names the columns and not the record, so it is the one thing inference cannot recover.
- `field` declared -> `read_field` answers it without touching the handle, so nothing is recovered from the bytes; a batch read still opens the stream and projects against it ([Options](options.md#edges)).

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test ipc -- mod_::internal
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/ipc
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_init.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/records.test.js
    ```
