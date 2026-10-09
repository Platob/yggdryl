# Arrow IPC

The Arrow IPC streaming format: one schema message, then one record-batch message per batch, so a stream carries its own schema and reads back batch by batch.

## Overview

| | |
| --- | --- |
| Declared by | `application/vnd.apache.arrow.stream`, `.arrows` |
| Build | default |
| Rust | `yggdryl::ipc`: `Ipc<H>` over any handle, configured by `IpcOptions`, `IPC_CODEC`, the medium's [codec](index.md#registering-a-medium), and the free `read_field`, `read_batch_reader` and `overwrite_arrow_reader` over any `IOBase` |
| Python, JavaScript | any `IOBase` whose name declares the stream, through the [calls every medium answers](index.md#read) |
| Settings | the shared [`RecordOptions`](index.md#options) and nothing of its own; a coding suffix such as `.arrows.gz` compresses the whole stream ([Compression](compression.md)) |
| Layouts | a union column crosses as itself, where [Parquet](parquet.md) refuses one by name |

## Read

The stream carries its schema, so a read needs no declaration, and `read_arrow_field` stops at the schema message before any batch body. A narrower `field` is a column pushdown: skipped columns are never decoded. A stored batch reads back as long as its writer made it unless `batch_row_size` or `batch_byte_size` bounds the read, which cuts it into views over its own buffers. A stream that is not there reads as no batches.

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

## Write

An overwrite writes the schema message, then one record-batch message per batch it is given, so the batches read back as they were written - and a reader with no batches still writes the schema. An append or a merge reads the stored stream and publishes it again with its rows, as every leaf does.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, IOMedia, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let schema = field.clone().into_arrow_schema()?;
    let ids = |values: Vec<i64>| {
        RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(Int64Array::from(values))])
    };

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;

    // A write with no batches still writes the schema the stream carries.
    handle.overwrite_arrow_reader(arrow::batch_reader(Arc::clone(&schema), []), &options)?;
    assert_eq!(handle.read_arrow_field(&options)?.field_len(), 1);
    assert_eq!(handle.read_arrow_reader(&options)?.count(), 0);

    // One record-batch message per batch written, read back as written.
    handle.overwrite_arrow_reader(
        arrow::batch_reader(Arc::clone(&schema), [ids(vec![1, 2])?, ids(vec![3])?]),
        &options,
    )?;
    let lengths = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.map(|batch| batch.num_rows()))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(lengths, [2, 1]);

    // An append keeps what is stored and adds its rows after it.
    handle.append_arrow_reader(arrow::batch_reader(Arc::clone(&schema), [ids(vec![4])?]), &options)?;
    assert_eq!(handle.row_size()?, 4);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "ids.arrows")

    # A write with no batches still writes the schema the stream carries.
    handle.overwrite_arrow_reader(pa.RecordBatchReader.from_batches(schema, []))
    assert [child.name for child in handle.read_arrow_field().dtype] == ["id"]
    assert handle.read_arrow_reader().read_all().num_rows == 0

    # One record-batch message per batch written, read back as written.
    batches = [
        pa.record_batch({"id": [1, 2]}, schema=schema),
        pa.record_batch({"id": [3]}, schema=schema),
    ]
    handle.overwrite_arrow_reader(pa.RecordBatchReader.from_batches(schema, batches))
    assert [batch.num_rows for batch in handle.read_arrow_reader()] == [2, 1]

    # An append keeps what is stored and adds its rows after it.
    handle.append_arrow_batch(pa.record_batch({"id": [4]}, schema=schema))
    assert handle.read_arrow_reader().read_all().column("id").to_pylist() == [1, 2, 3, 4]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const ids = (values) =>
      new arrow.Table({ id: arrow.vectorFromArray(values, new arrow.Int64()) }).batches[0]

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM

    // A write with no batches still writes the schema the stream carries.
    handle.overwriteArrowTable(new arrow.Table(new arrow.Schema([new arrow.Field('id', new arrow.Int64(), false)])))
    assert.deepEqual([...handle.readArrowField().dtype].map((child) => child.name), ['id'])
    assert.equal(handle.readArrowReader().intoTable().numRows, 0)

    // One record-batch message per batch written, read back as written.
    handle.overwriteArrowReader(BatchReader.from([ids([1n, 2n]), ids([3n])]))
    assert.deepEqual([...handle.readArrowReader()].map((batch) => batch.numRows), [2, 1])

    // An append keeps what is stored and adds its rows after it.
    handle.appendArrowBatch(ids([4n]))
    assert.equal(handle.rowSize(), 4)
    assert.deepEqual([...handle.readArrowReader().intoTable().getChild('id')], [1n, 2n, 3n, 4n])
    ```

## Performance

Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150, rustc 1.96.1 (2026-08-23). The read fixture holds 65,536 rows and four columns; the write fixture holds 4,096 rows, with the stored side prepared outside the timer.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 4.33 ms | 15.1M rows/s |
| `overwrite_arrow_reader` | 4,096 | 181 us | 22.6M rows/s |
| `append_arrow_reader` | 4,096 | 615 us | 6.66M rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 5.44 ms | 754k rows/s |

The same 65,536-row fixture, closed against opened:

| dimension | fresh | opened |
| --- | ---: | ---: |
| `row_size` | 2.51 us | 6.63 ns |
| `column_size` | 6.25 us | 7.04 ns |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/ipc
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/ipc
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_record
```

`python/benchmarks/media.py` carries a PyArrow IPC write baseline over the same batches and sink. One containerized x86_64 Linux run with `--min-time 0.1 --repeat 3`, 65,536 rows, 4 columns, 8 batches.

```text
ipc write reader                 1.133 ms   57.9M rows/s
PyArrow IPC write baseline       1.607 ms   40.8M rows/s
```

```bash
python/.venv/bin/python python/benchmarks/media.py --filter ipc --filter "PyArrow IPC"
```

Through the `Media` enum, whose `Ipc` variant redirects to the same implementation:

| operation through `Media::Ipc` | estimate | throughput |
| --- | ---: | ---: |
| overwrite | 82.2 us | 49.8M rows/s |
| append | 424 us | 9.67M rows/s |
| keyed merge (upsert) | 6.41 ms | 639k rows/s |

Criterion prepares the stored side for append and merge outside the timer. Sub-millisecond estimates are regression anchors.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/media_ipc
```
