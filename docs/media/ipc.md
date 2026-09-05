# Arrow IPC

`yggdryl::media::ipc` reads and writes Arrow IPC streams over any byte handle.

## Contract

| Key | Value |
| --- | --- |
| Owns | `ipc::read_field`, `ipc::read_batch_reader`, `ipc::overwrite_arrow_reader`, `Ipc<H>`, `IpcOptions` |
| Handle surface | `overwrite_*`, `append_*`, keyed `merge_*`, `read_arrow_reader`, `read_arrow_field` from [`IOMedia`](../holder/iobase/records.md) |
| Merge | `merge_by_names` supplies identity only; the method name carries intent, never the key |
| Schema | self-describing; `dtype` set skips the handle; root name defaults to `DEFAULT_ROOT_NAME` (`"row"`) |
| Pushdown | `field`, a non-null struct root naming a subset, projects at decode; keeps stored order and types, never casts |
| Coding | last content coding of the handle media type (`.gz`, `.zst`); `level` is the only compression setting |
| Cached | `open` caches schema and dimensions until `close`; writes and every `Ipc` builder drop the cache |
| Format settings | none beyond the shared [`IORecordOptions`](options.md) fields |
| Errors | bytes that are not a stream fail `read_field` and `read_batch_reader` at once |
| Bindings | Rust: free functions and `Ipc<H>`; Python: `IOBase` with `pyarrow.RecordBatchReader` over Arrow C Stream; JavaScript: `IOBase` with Arrow JS over the copied [IPC boundary](../extensions/javascript.md) |

## Use

Append retains stored rows; keyed merge updates matching `id` values and inserts misses.

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

    # The name says Arrow IPC, so no call names an encoding.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(batch([1, 2], ["XNAS", "XNYS"]))
    handle.append_arrow_batch(batch([3], ["XLON"]))

    merging = handle.record_options()
    merging.merge_by_names = ["id"]
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
      handle.recordOptions().withMergeByNames(['id']),
    )

    assert.equal(handle.readArrowField().name, 'row')
    assert.equal(handle.readArrowReader().intoTable().numRows, 4)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Dimensions and opened sessions

`row_size` counts IPC message metadata and skips dictionary and record-batch bodies; `column_size` reads the canonical Struct field. Both describe the whole stream and ignore selection, partition filters, and read limits.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::holder::Holder;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int64Array::from(vec![1, 2]))],
    )?;
    let mut handle = Holder::buffer(Buffer::new().with_media_type(MimeType::ARROW_STREAM.into()));
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch]), &options)?;

    handle.open()?;
    assert_eq!(handle.read_arrow_field(&options)?, field);
    assert_eq!((handle.row_size()?, handle.column_size()?), (2, 1));
    handle.close()?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "dimensions.arrows")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2]}))
    with handle:
        assert (handle.row_size, handle.column_size) == (2, 1)
        assert handle.read_arrow_field().name == "row"
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
    const handle = new IOBase(path.join(root, 'dimensions.arrows'))
    handle.overwriteArrowTable(arrow.tableFromArrays({ id: [1, 2] }))
    handle.open()
    assert.deepEqual([handle.rowSize, handle.columnSize], [2, 1])
    assert.equal(handle.readArrowField().name, 'row')
    handle.close()
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Reading and writing are both readers

`ipc::read_batch_reader` returns [`arrow::BatchReader`](../arrow/readers.md), an iterator whose schema is known before the first batch. Batches come back as written, block boundaries included.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.into_arrow_schema()?;
    let batches = (0..3)
        .map(|start| {
            RecordBatch::try_new(
                arrow_schema.clone(),
                vec![Arc::new(Int64Array::from(vec![start, start + 1]))],
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = IpcOptions::new();

    // `batch_reader` turns whatever is already in hand - a Vec, an array, an
    // iterator - into the one shape a write takes.
    ipc::overwrite_arrow_reader(
        &mut handle,
        arrow::batch_reader(arrow_schema, batches),
        &options,
    )?;

    let reader = ipc::read_batch_reader(&handle, None, &options)?;
    // The schema is known before a single batch is decoded.
    assert_eq!(reader.schema().fields().len(), 1);

    let mut rows = 0;
    for batch in reader {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 6);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    batches = [
        pa.record_batch({"id": [start, start + 1]}, schema=schema)
        for start in range(0, 6, 2)
    ]

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")

    # The primitive write consumes exactly one RecordBatchReader.
    handle.overwrite_arrow_reader(pa.RecordBatchReader.from_batches(schema, batches))

    reader = handle.read_arrow_reader()
    # The schema is known before a single batch is decoded.
    assert reader.schema.names == ["id"]

    rows = sum(batch.num_rows for batch in reader)
    assert rows == 6
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const batches = [0, 2, 4].map(
      (start) =>
        new arrow.Table({
          id: arrow.vectorFromArray([BigInt(start), BigInt(start + 1)], new arrow.Int64()),
        }).batches[0],
    )

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM

    // An Arrow JS Table, one RecordBatch, an array of them, or Arrow IPC bytes:
    // `BatchReader.from` turns whatever is in hand into the shape a write takes.
    handle.overwriteArrowReader(BatchReader.from(batches))

    const reader = handle.readArrowReader()
    // The schema is known before a single batch is decoded.
    assert.deepEqual([...reader.field.dtype].map((child) => child.name), ['id'])

    let rows = 0
    for (const batch of reader) rows += batch.numRows
    assert.equal(rows, 6)
    ```

`ipc::overwrite_arrow_reader` encodes each batch as it pulls it, so a lazy reader is never materialized.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.into_arrow_schema()?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());

    // Nothing is materialized: each batch is built as the writer asks for it.
    let produced = (0..4).map({
        let arrow_schema = arrow_schema.clone();
        move |start| {
            RecordBatch::try_new(
                arrow_schema.clone(),
                vec![Arc::new(Int64Array::from(vec![start]))],
            )
            .expect("batch")
        }
    });
    ipc::overwrite_arrow_reader(
        &mut handle,
        arrow::batch_reader(arrow_schema, produced),
        &IpcOptions::new(),
    )?;

    assert_eq!(
        ipc::read_batch_reader(&handle, None, &IpcOptions::new())?.count(),
        4
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")

    # Nothing is materialized: each batch is built as the writer asks for it.
    produced = (
        pa.record_batch({"id": [start]}, schema=schema) for start in range(4)
    )
    handle.overwrite_arrow_reader(pa.RecordBatchReader.from_batches(schema, produced))

    assert sum(1 for _ in handle.read_arrow_reader()) == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM

    // Apache Arrow JS owns the encoding of what a caller already holds, so the
    // four batches cross the boundary once, as one Arrow IPC stream.
    const produced = [0, 1, 2, 3].map(
      (start) =>
        new arrow.Table({ id: arrow.vectorFromArray([BigInt(start)], new arrow.Int64()) })
          .batches[0],
    )
    handle.overwriteArrowReader(BatchReader.from(produced))

    assert.equal([...handle.readArrowReader()].length, 4)
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
        DataType::Utf8.required_field("symbol"),
        DataType::Utf8.required_field("venue"),
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

The `field` argument is a column pushdown and nothing else: skipped columns are never decoded or allocated. An IPC batch body is one message and is still read whole; [Parquet](parquet.md) is where a projection also removes reading.

## One stream, one configuration

Rust only.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;
use yggdryl::media::ipc::Ipc;
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

let handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
let mut media = Ipc::new(handle).with_field(schema.clone());
let options = media.record_options()?;

// One options value carries the schema, root name, and coding.
media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
assert_eq!(media.read_arrow_reader(&options)?.count(), 1);
assert_eq!(media.read_arrow_field(&options)?, schema);

// An Ipc is also the bytes it encodes: a stream opens with its continuation marker.
assert_eq!(media.read_range_bytes(0, 4)?, [0xFF, 0xFF, 0xFF, 0xFF]);
```

`Ipc<H>` implements `IOBase` by delegating to the handle it owns, which is why `read_range_bytes` works on it and why [`Media::Ipc`](index.md) can hold it.

| `Ipc<H>` call | Effect |
| --- | --- |
| `record_options` | the defaults as the `RecordOptions` every canonical `IOMedia` call accepts |
| `handle`, `handle_mut`, `into_handle` | reach the wrapped handle |
| `options`, `options_mut` | change future defaults |
| `with_options` | replaces the whole settings value |
| `with_field`, `with_name` | reach the declared root |
| any `with_*`, `with_level` included | drops the opened metadata cache |

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

## Content coding comes from the name

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::Ipc;
    use yggdryl::{DataType, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;

    let mut sizes = Vec::new();
    for name in ["trades.arrows", "trades.arrows.gz", "trades.arrows.zst"] {
        let url = Url::from_str(&format!("file:///{name}"))?;
        let handle = Buffer::new().with_media_type(url.media_type());
        let mut media = Ipc::new(handle).with_field(schema.clone());

        let batch = RecordBatch::try_new(
            arrow_schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2]))],
        )?;
        let options = media.record_options()?;
        media.overwrite_arrow_reader(
            arrow::batch_reader(arrow_schema.clone(), [batch]),
            &options,
        )?;

        // Identical calls on both sides, whatever the coding is.
        assert_eq!(media.read_arrow_reader(&options)?.count(), 1, "{name}");
        sizes.push(media.handle().as_slice().to_vec());
    }

    // The bytes underneath are framed by the coding the name declared.
    assert_eq!(&sizes[1][..2], &[0x1F, 0x8B]);
    assert_eq!(&sizes[2][..4], &[0x28, 0xB5, 0x2F, 0xFD]);
    assert_ne!(sizes[0], sizes[1]);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp())

    written = []
    for name in ("trades.arrows", "trades.arrows.gz", "trades.arrows.zst"):
        handle = IOBase(root / name)
        handle.overwrite_arrow_batch(pa.record_batch({"id": [1, 2]}, schema=schema))

        # Identical calls on both sides, whatever the coding is.
        assert handle.read_arrow_reader().read_all().num_rows == 2, name
        written.append(handle.read_bytes())

    # The bytes underneath are framed by the coding the name declared.
    assert written[1][:2] == bytes.fromhex("1f8b")
    assert written[2][:4] == bytes.fromhex("28b52ffd")
    assert written[0] != written[1]
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
    const written = []
    for (const name of ['trades.arrows', 'trades.arrows.gz', 'trades.arrows.zst']) {
      const handle = new IOBase(path.join(root, name))
      handle.overwriteArrowTable(
        new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) }),
      )

      // Identical calls on both sides, whatever the coding is.
      assert.equal(handle.readArrowReader().intoTable().numRows, 2, name)
      written.push(handle.readBytes())
    }

    // The bytes underneath are framed by the coding the name declared.
    assert.deepEqual([...written[1].subarray(0, 2)], [0x1f, 0x8b])
    assert.deepEqual([...written[2].subarray(0, 4)], [0x28, 0xb5, 0x2f, 0xfd])
    assert.notDeepEqual(written[0], written[1])

    fs.rmSync(root, { recursive: true, force: true })
    ```

`IOBase::codec` reads the last content coding out of the media type; the encoding applies it on write and strips it on read. `trades.arrows.gz` round-trips through [gzip](../coding/gzip.md), `trades.arrows.zst` through [zstd](../coding/zstd.md), with identical calls.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::Ipc;
    use yggdryl::{DataType, Level, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from((0..512).collect::<Vec<i64>>()))],
    )?;

    let handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows.gz")?.media_type());
    let mut media = Ipc::new(handle)
        .with_field(schema.clone())
        .with_level(Level::BEST);
    let options = media.record_options()?;

    media.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )?;
    assert_eq!(media.read_arrow_reader(&options)?.count(), 1);
    // Still a gzip member, and smaller than the stream it encodes.
    assert_eq!(&media.handle().as_slice()[..2], &[0x1F, 0x8B]);
    assert!(media.handle().size() < 512 * 8);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows.gz")

    options = handle.record_options()
    options.level = 9
    handle.overwrite_arrow_batch(
        pa.record_batch({"id": list(range(512))}, schema=schema), options=options
    )

    assert handle.read_arrow_reader().read_all().num_rows == 512
    # Still a gzip member, and smaller than the stream it encodes.
    assert handle.read_bytes()[:2] == bytes.fromhex("1f8b")
    assert handle.size < 512 * 8
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
    const handle = new IOBase(path.join(root, 'trades.arrows.gz'))

    const ids = Array.from({ length: 512 }, (_, index) => BigInt(index))
    handle.overwriteArrowTable(
      new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
      handle.recordOptions().withLevel(9),
    )

    assert.equal(handle.readArrowReader().intoTable().numRows, 512)
    // Still a gzip member, and smaller than the stream it encodes.
    assert.deepEqual([...handle.readBytes().subarray(0, 2)], [0x1f, 0x8b])
    assert.ok(handle.size < 512 * 8)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Options

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions, DEFAULT_ROOT_NAME};
    use yggdryl::media::ipc::IpcOptions;
    use yggdryl::{DataType, Level, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let options = IpcOptions::new()
        .with_field(schema.clone())
        .with_level(Level::BEST);

    assert_eq!(options.field(), Some(schema.clone()));
    assert_eq!(options.name(), DEFAULT_ROOT_NAME);
    assert_eq!(options.dtype(), Some(schema.dtype()));
    assert_eq!(options.level(), Level::BEST);

    // The fields are public, so a setting can also be written directly.
    let mut direct = IpcOptions::new();
    direct.batch_row_size = Some(1024);
    assert_eq!(direct.batch_row_size(), Some(1024));

    // It converts into the enum every encoding's settings share.
    let erased: RecordOptions = options.into();
    assert_eq!(erased.mime_type(), MimeType::ARROW_STREAM);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import RecordOptions

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    # The media type names the encoding, so there is no format argument.
    options = RecordOptions("trades.arrows")
    options.field = schema
    options.level = 9

    assert options.field is not None
    assert options.name == "row"
    assert options.level == 9

    options.batch_row_size = 1024
    assert options.batch_row_size == 1024

    assert str(options.mime_type) == "application/vnd.apache.arrow.stream"
    # A setting another encoding has is absent rather than invented here.
    assert options.max_row_group_size is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, RecordOptions, fields } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    // The media type names the encoding, so there is no format argument.
    const options = new RecordOptions('trades.arrows')
    options.field = schema
    options.level = 9

    assert.ok(options.field.equals(schema))
    assert.equal(options.name, 'row')
    assert.equal(options.level, 9)

    options.batchRowSize = 1024
    assert.equal(options.batchRowSize, 1024)

    assert.equal(options.mimeType.toString(), 'application/vnd.apache.arrow.stream')
    // `with*` returns a new value rather than changing the one it was built from.
    assert.equal(options.withSafe(true).safe, true)
    assert.equal(options.safe, false)
    ```

`IpcOptions` holds the shared record settings as public fields: `name`, `dtype`, `metadata`, `safe`, `batch_row_size`, `max_row_size`, `max_byte_size`, `commit_row_size`, `level`, `merge_by_names`, `select_by_names`, and `filter_partitions`. The `ipc::*` functions handle only the encoding seam; the [`IOMedia`](../holder/iobase/records.md) path adds casting, re-chunking, selection, limits, partition filters, commit cadence, and write intent.

## Absence

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::Ipc;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // A resource that does not exist yet holds no batches; it is not a parse failure.
    let missing = Ipc::new(Buffer::new()).with_field(schema.clone());
    let options = missing.record_options()?;
    let reader = missing.read_arrow_reader(&options)?;
    // The declared schema is what the empty reader reports.
    assert_eq!(reader.schema().fields().len(), 1);
    assert_eq!(reader.count(), 0);

    // Opening an absent stream succeeds and caches explicit zero dimensions.
    let mut empty = Ipc::new(Buffer::new());
    empty.open()?;
    assert!(empty.opened());
    assert_eq!(empty.row_size()?, 0);
    assert_eq!(empty.column_size()?, 0);

    // Writing no batches still writes the schema, so the stream exists and is readable.
    let mut written = Ipc::new(Buffer::new()).with_field(schema.clone());
    let options = written.record_options()?;
    written.overwrite_arrow_reader(
        arrow::batch_reader(
            schema.clone().into_arrow_schema()?,
            std::iter::empty::<RecordBatch>(),
        ),
        &options,
    )?;
    assert!(!written.handle().is_empty());
    assert_eq!(written.read_arrow_reader(&options)?.count(), 0);
    assert_eq!(written.read_arrow_field(&options)?, schema);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp())

    # A resource that does not exist yet holds no batches; it is not a parse failure.
    missing = IOBase(root / "missing.arrows")
    assert not missing.exists()
    assert missing.read_arrow_reader().read_all().num_rows == 0

    # Writing no batches still writes the schema, so the stream exists and reads.
    written = IOBase(root / "empty.arrows")
    written.overwrite_arrow_table(pa.Table.from_batches([], schema=schema))
    assert written.size > 0
    assert written.read_arrow_reader().read_all().num_rows == 0
    assert written.read_arrow_field().name == "row"
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

    // A resource that does not exist yet holds no batches; it is not a parse failure.
    const missing = new IOBase(path.join(root, 'missing.arrows'))
    assert.ok(!missing.exists())
    assert.equal(missing.readArrowReader().intoTable().numRows, 0)

    // Writing no batches still writes the schema, so the stream exists and reads.
    const schema = new arrow.Schema([new arrow.Field('id', new arrow.Int64(), true)])
    const written = new IOBase(path.join(root, 'empty.arrows'))
    written.overwriteArrowTable(new arrow.Table(schema))
    assert.ok(written.size > 0)
    assert.equal(written.readArrowReader().intoTable().numRows, 0)
    assert.equal(written.readArrowField().name, 'row')

    fs.rmSync(root, { recursive: true, force: true })
    ```

A location that holds nothing yields nothing, the laziness rule [Bytes](../holder/iobase/bytes.md) sets. Anything that is not a stream fails on the spot.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::ipc::{self, IpcOptions};

    let handle = Buffer::from_bytes(b"definitely not an Arrow IPC stream".to_vec());
    assert!(ipc::read_field(&handle, &IpcOptions::new()).is_err());
    assert!(ipc::read_batch_reader(&handle, None, &IpcOptions::new()).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import IOBase

    handle = IOBase.from_bytes(b"definitely not an Arrow IPC stream")
    handle.media_type = "application/vnd.apache.arrow.stream"

    with pytest.raises(ValueError):
        handle.read_arrow_field()
    with pytest.raises(ValueError):
        handle.read_arrow_reader()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes(Buffer.from('definitely not an Arrow IPC stream'))
    handle.mediaType = MimeType.ARROW_STREAM

    assert.throws(() => handle.readArrowField(), /Arrow/)
    assert.throws(() => handle.readArrowReader(), /Arrow/)
    ```

[Parquet](parquet.md) has the same `read_field`, `read_batch_reader`, and `overwrite_arrow_reader` shape behind the non-default `parquet` feature.

## Edges

- `merge_by_names` on an overwrite or append -> the key never selects merge; intent stays with the method name.
- `append_*` or keyed `merge_*` -> published through `ipc::overwrite_arrow_reader`, the one complete-stream encoder.
- `field` naming every stored column, or one the stream lacks -> reads everything; a projection only drops columns.
- projected read -> the returned reader reports the projected schema; Arrow's own `StreamReader` would report the whole stream's.
- reading -> batches arrive one at a time and only the current one is alive.
- `IpcOptions::dtype` set -> `read_field` builds the field without touching the handle.
- `field` accessor -> built from `name`, `dtype`, and `metadata` by [`IORecordOptions`](options.md).
- handle with no content coding -> `level` does nothing.
- missing resource -> zero batches, not a parse failure; the reader reports the declared schema, or an empty Arrow schema without one.
- `open` on an absent stream -> succeeds and caches explicit zero dimensions.
- zero batches written -> the schema is still written; the stream exists and answers its schema from the bytes.
- bytes that are not a stream -> `read_field` and `read_batch_reader` error; Python raises `ValueError`, JavaScript throws matching `/Arrow/`.
- closed handle -> every call reads fresh metadata; `open` retains the inferred IPC wrapper.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib media::ipc::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/ipc
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/ipc
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_record
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/ipc
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_ipc.py
    python/.venv/bin/python python/benchmarks/media.py --filter ipc --filter "PyArrow IPC"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    YGGDRYL_BENCH_FILTER=records/read_ipc npm run --prefix node bench:media
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
