# Arrow IPC

`yggdryl::ipc` reads and writes Arrow IPC streams over any byte handle.

!!! note "All three"
    Python and JavaScript reach the encoding through [`IOBase`](io.md)'s record
    methods rather than through the free functions. Python exchanges batches as
    `pyarrow.RecordBatchReader`, and JavaScript as Apache Arrow JS values over
    the copied Arrow IPC boundary described in
    [javascript.md](extensions/javascript.md).

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::io::Buffer;
    use yggdryl::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    // A non-null struct Field is the schema of the batches it describes.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let arrow_schema = schema.to_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = IpcOptions::new();

    ipc::write_batch_reader(&mut handle, arrow::batch_reader(arrow_schema, [batch]), &options)?;
    assert_eq!(ipc::read_field(&handle, &options)?, schema);
    assert_eq!(ipc::read_batch_reader(&handle, None, &options)?.count(), 1);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    batch = pa.record_batch({"id": [1, 2], "symbol": ["AAPL", None]}, schema=schema)

    # The name says Arrow IPC, so no call names an encoding.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.write_arrow_batch_reader(batch)

    assert handle.read_arrow_field().name == "row"
    assert handle.read_arrow_batch_reader().read_all() == pa.Table.from_batches([batch])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
    })

    // The name says Arrow IPC, so no call names an encoding.
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.arrows'))
    handle.writeArrowBatchReader(table)

    assert.equal(handle.readArrowField().name, 'row')
    assert.equal(handle.readArrowBatchReader().toTable().numRows, 2)

    fs.rmSync(root, { recursive: true, force: true })
    ```

Three free functions are the whole encoding. `read_field` answers what the stream holds,
`ipc::read_batch_reader` yields the batches, and `ipc::write_batch_reader` replaces the stream. Each
takes an [`IOBase`](io.md) handle and one `IpcOptions`, and nothing else: no path, no file,
no codec argument. A [`Buffer`](io.md) here, a [`local::File`](local.md) in a program, the
same three calls.

Streaming is the only shape: a read returns an [`arrow::BatchReader`](arrow.md) and a write
consumes one. The reader a write is handed carries its own Arrow schema, and that schema is
what the stream declares, so a caller replacing a stream with the non-null struct root
`Field` of [field.md](field.md) projects that root and builds the reader over it -
`arrow::batch_reader` is the constructor that turns batches a caller already has into one.
`ipc::write_batch_reader` replaces the stream rather than appending to it.

Python spells the same two operations `IOBase.read_arrow_batch_reader` and
`IOBase.write_arrow_batch_reader`, and its reader is a `pyarrow.RecordBatchReader`: batches cross
through the Arrow C Stream interface in both directions, so neither side copies or rebuilds
them. `IOBase.read_arrow_field` is `read_field`, and the encoding still comes from the
handle's media type rather than from an argument.

There is no row-level read or write. A batch is the unit at every level of this module.

## Reading and writing are both readers

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::io::Buffer;
    use yggdryl::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.to_arrow_schema()?;
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
    ipc::write_batch_reader(
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

    # A list, a Table, a RecordBatch, or a RecordBatchReader: anything PyArrow
    # exports an Arrow C stream from is the one shape a write takes.
    handle.write_arrow_batch_reader(batches)

    reader = handle.read_arrow_batch_reader()
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
    handle.writeArrowBatchReader(BatchReader.from(batches))

    const reader = handle.readArrowBatchReader()
    // The schema is known before a single batch is decoded.
    assert.deepEqual([...reader.field.dataType].map((child) => child.name), ['id'])

    let rows = 0
    for (const batch of reader) rows += batch.numRows
    assert.equal(rows, 6)
    ```

`ipc::read_batch_reader` returns [`arrow::BatchReader`](arrow.md), a boxed `RecordBatchReader`.
It is an iterator, so batches arrive one at a time and only the current one is alive; the
stream's Arrow schema is available from the reader itself, ahead of the first batch.

Batches come back exactly as they were written. `ipc::read_batch_reader` does not cast them to
a declared schema, so what goes in is what comes out, block boundaries included.

The write side is the same type facing the other way: `ipc::write_batch_reader` consumes a
`BatchReader` and encodes each batch as it pulls it, so a reader that computes its batches
lazily is never materialized.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::io::Buffer;
    use yggdryl::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.to_arrow_schema()?;

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
    ipc::write_batch_reader(
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
    handle.write_arrow_batch_reader(pa.RecordBatchReader.from_batches(schema, produced))

    assert sum(1 for _ in handle.read_arrow_batch_reader()) == 4
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
    handle.writeArrowBatchReader(BatchReader.from(produced))

    assert.equal([...handle.readArrowBatchReader()].length, 4)
    ```

## Column pushdown

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::io::Buffer;
    use yggdryl::ipc::{self, IpcOptions};
    use yggdryl::{DataType, MimeType};

    let stored = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Utf8.required_field("venue"),
    ])?
    .required_field("row");
    let arrow_schema = stored.to_arrow_schema()?;

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
    ipc::write_batch_reader(&mut handle, arrow::batch_reader(arrow_schema, [batch]), &options)?;

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
    handle.write_arrow_batch_reader(batch)

    # One of the three columns, declared as this read's schema - a single
    # setting is its own keyword, no options object needed.
    projected = handle.read_arrow_batch_reader(
        schema=pa.schema([pa.field("id", pa.int64(), nullable=False)])
    )
    assert projected.schema.names == ["id"]
    assert projected.read_all().num_columns == 1

    # The stream itself is unchanged: it still carries all three.
    assert len(handle.read_arrow_field().data_type) == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, IOBase, MimeType, fields } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.writeArrowBatchReader(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
        venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
      }),
    )

    // One of the three columns, declared as this read's schema.
    const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    const projected = handle.readArrowBatchReader(handle.recordOptions().withSchema(wanted))
    assert.deepEqual([...projected.field.dataType].map((child) => child.name), ['id'])
    assert.equal(projected.toTable().numCols, 1)

    // The stream itself is unchanged: it still carries all three.
    assert.equal(handle.readArrowField().dataType.length, 3)
    ```

The `field` argument to `ipc::read_batch_reader` is a column pushdown and nothing else. A
non-null struct root naming a subset of the stored columns becomes the projection the Arrow
IPC decoder takes, so the columns it leaves out are never turned into arrays. Be precise
about what that saves: an IPC record batch is one contiguous message, so its body is still
read off the handle whole - the projection removes the decode and the allocation, not the
bytes. [parquet.md](parquet.md), whose column chunks are separately addressable, is where a
projection also removes reading.

A root naming every stored column, or naming one the stream does not carry, reads
everything: a projection can only drop columns, never invent them. The selection keeps the
stored order and the stored types. The handle-level `read_arrow_batch_reader` in
[io.md](io.md) is the one that also casts: it declares this schema, gets the projection out
of it, and then reshapes what comes back.

Arrow's own projected `StreamReader` reports the whole stream's schema while yielding
projected batches. The reader returned here reports the projected schema, so what it says
and what it yields agree.

## One stream, one configuration

!!! note "Rust only"
    Python and JavaScript reach the encoding through the handle itself; the
    stateful wrapper is a Rust type.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::ipc::Ipc;
use yggdryl::{DataType, Url};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.to_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

let handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
let mut media = Ipc::new(handle).with_schema(schema.clone());

// No call repeats the schema, the root name, or the coding.
media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;
assert_eq!(media.read_batch_reader(None)?.count(), 1);
assert_eq!(media.schema()?, schema);

// An Ipc is also the bytes it encodes: a stream opens with its continuation marker.
assert_eq!(media.read_range(0, 4)?, [0xFF, 0xFF, 0xFF, 0xFF]);
```

`Ipc<H>` is the same encoding with the handle, the options, and the cached schema held in
one place instead of being passed at every call. `handle`, `handle_mut`, and
`into_handle` reach the wrapped handle; `options` and `options_mut` reach the settings.

`Ipc<H>` implements `IOBase` by delegating to the handle it owns, which is why
`read_range` above works on it directly. That is what lets a stream be copied, compressed,
or handed to another reader without unwrapping it first, and what lets an `Ipc` be held as
[`generic::Media::Ipc`](generic.md).

## The stream carries its schema

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::io::Buffer;
    use yggdryl::ipc::{Ipc, DEFAULT_ROOT_NAME};
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.to_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from(vec![7]))],
    )?;

    let mut writer = Ipc::new(Buffer::new()).with_schema(schema.clone());
    writer.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;
    let bytes = writer.handle().as_slice().to_vec();

    // A reader that declares nothing recovers the schema from the bytes.
    let mut reader = Ipc::new(Buffer::from_bytes(bytes.clone()));
    assert_eq!(reader.schema()?, schema);
    assert_eq!(reader.schema()?.name(), DEFAULT_ROOT_NAME);

    // Arrow names columns, not the record; the root name is chosen on this side.
    let mut named = Ipc::new(Buffer::from_bytes(bytes)).with_root_name("trade");
    assert_eq!(named.schema()?.name(), "trade");
    assert_eq!(named.schema()?.get_field_by_name("id"), schema.get_field_by_name("id"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.write_arrow_batch_reader(pa.record_batch({"id": [7]}, schema=schema))

    # A reader that declares nothing recovers the schema from the bytes.
    assert handle.read_arrow_field().name == "row"

    # Arrow names columns, not the record; the root name is chosen on this side.
    named = handle.record_options()
    named.root_name = "trade"
    assert handle.read_arrow_field(options=named).name == "trade"
    assert [child.name for child in handle.read_arrow_field().data_type] == ["id"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.writeArrowBatchReader(
      new arrow.Table({ id: arrow.vectorFromArray([7n], new arrow.Int64()) }),
    )

    // A reader that declares nothing recovers the schema from the bytes.
    assert.equal(handle.readArrowField().name, 'row')

    // Arrow names columns, not the record; the root name is chosen on this side.
    const named = handle.recordOptions().withRootName('trade')
    assert.equal(handle.readArrowField(named).name, 'trade')
    assert.deepEqual([...handle.readArrowField().dataType].map((child) => child.name), ['id'])
    ```

An IPC stream is self-describing, so a declared schema is never required to read one. When
`IpcOptions::schema` is set, `read_field` returns it without touching the handle; when it
is absent, the stream's Arrow schema is converted back to a `Field` and the struct root
is named `root_name`, which defaults to `DEFAULT_ROOT_NAME` - `"row"`. Arrow carries names
for the columns and none for the record, so that one name is the only thing inference
cannot recover.

## Content coding comes from the name

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::io::Buffer;
    use yggdryl::ipc::Ipc;
    use yggdryl::{DataType, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.to_arrow_schema()?;

    let mut sizes = Vec::new();
    for name in ["trades.arrows", "trades.arrows.gz", "trades.arrows.zst"] {
        let url = Url::from_str(&format!("file:///{name}"))?;
        let handle = Buffer::new().with_media_type(url.media_type());
        let mut media = Ipc::new(handle).with_schema(schema.clone());

        let batch = RecordBatch::try_new(
            arrow_schema.clone(),
            vec![Arc::new(Int64Array::from(vec![1, 2]))],
        )?;
        media.write_batch_reader(arrow::batch_reader(arrow_schema.clone(), [batch]))?;

        // Identical calls on both sides, whatever the coding is.
        assert_eq!(media.read_batch_reader(None)?.count(), 1, "{name}");
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
        handle.write_arrow_batch_reader(pa.record_batch({"id": [1, 2]}, schema=schema))

        # Identical calls on both sides, whatever the coding is.
        assert handle.read_arrow_batch_reader().read_all().num_rows == 2, name
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
      handle.writeArrowBatchReader(
        new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) }),
      )

      // Identical calls on both sides, whatever the coding is.
      assert.equal(handle.readArrowBatchReader().toTable().numRows, 2, name)
      written.push(handle.readBytes())
    }

    // The bytes underneath are framed by the coding the name declared.
    assert.deepEqual([...written[1].subarray(0, 2)], [0x1f, 0x8b])
    assert.deepEqual([...written[2].subarray(0, 4)], [0x28, 0xb5, 0x2f, 0xfd])
    assert.notDeepEqual(written[0], written[1])

    fs.rmSync(root, { recursive: true, force: true })
    ```

Compression is never an argument here. The handle reports a media type, `IOBase::codec`
reads the last content coding out of it, and the encoding applies it on write and strips
it on read. A handle named `trades.arrows.gz` round-trips through [gzip](gzip.md), one
named `trades.arrows.zst` through [zstd](zstd.md), and the calls above do not change.

`IpcOptions::level` is the only compression setting, and it reaches whichever coding the
handle declared. It does nothing when the handle declares none.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::ipc::Ipc;
    use yggdryl::{DataType, Level, Url};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = schema.to_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from((0..512).collect::<Vec<i64>>()))],
    )?;

    let handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows.gz")?.media_type());
    let mut media = Ipc::new(handle)
        .with_schema(schema.clone())
        .with_level(Level::BEST);

    media.write_batch_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch]))?;
    assert_eq!(media.read_batch_reader(None)?.count(), 1);
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
    handle.write_arrow_batch_reader(
        pa.record_batch({"id": list(range(512))}, schema=schema), options=options
    )

    assert handle.read_arrow_batch_reader().read_all().num_rows == 512
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
    handle.writeArrowBatchReader(
      new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
      handle.recordOptions().withLevel(9),
    )

    assert.equal(handle.readArrowBatchReader().toTable().numRows, 512)
    // Still a gzip member, and smaller than the stream it encodes.
    assert.deepEqual([...handle.readBytes().subarray(0, 2)], [0x1f, 0x8b])
    assert.ok(handle.size < 512 * 8)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Options

=== "Rust"

    ```rust
    use yggdryl::generic::{IORecordOptions, RecordOptions};
    use yggdryl::ipc::{IpcOptions, DEFAULT_ROOT_NAME};
    use yggdryl::{DataType, Level, MimeType};

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    let options = IpcOptions::new()
        .with_schema(schema.clone())
        .with_level(Level::BEST);

    assert_eq!(options.schema(), Some(&schema));
    assert_eq!(options.root_name(), DEFAULT_ROOT_NAME);
    assert_eq!(options.level(), Level::BEST);

    // The fields are public, so a setting can also be written directly.
    let mut direct = IpcOptions::new();
    direct.batch_size = Some(1024);
    assert_eq!(direct.batch_size(), Some(1024));

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
    options.schema = schema
    options.level = 9

    assert options.schema is not None
    assert options.root_name == "row"
    assert options.level == 9

    options.batch_size = 1024
    assert options.batch_size == 1024

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
    options.schema = schema
    options.level = 9

    assert.ok(options.schema.equals(schema))
    assert.equal(options.rootName, 'row')
    assert.equal(options.level, 9)

    options.batchSize = 1024
    assert.equal(options.batchSize, 1024)

    assert.equal(options.mimeType.toString(), 'application/vnd.apache.arrow.stream')
    // `with*` returns a new value rather than changing the one it was built from.
    assert.equal(options.withSafe(true).safe, true)
    assert.equal(options.safe, false)
    ```

`IpcOptions` holds the five settings shared by every record encoding - `schema`,
`root_name`, `safe`, `batch_size`, `level` - as public fields, and implements
[`IORecordOptions`](generic.md) over them, which is where the `with_*` builders and the
accessors come from. IPC adds nothing of its own: a stream carries its schema, and its
coding comes from the handle.

Two of those five are carried and not consulted here. `safe` governs a cast, and this
encoding performs none; `batch_size` bounds a reader that re-chunks, and IPC returns the
batches the stream was written with. They are on the type because a caller holding a
`RecordOptions` should not have to know which encoding is underneath.

`Ipc::with_options` replaces the whole settings value at once, and every builder on `Ipc`
that touches the schema or the root name drops the cached schema with it. `with_level` does
not: it changes how bytes are written, not what they say.

## The schema cache

!!! note "Rust only"
    The bindings' `open`/`close` cache the resource, not a decoded schema: the
    schema cache belongs to the `Ipc` wrapper.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::io::{Buffer, IOBase};
use yggdryl::ipc::Ipc;
use yggdryl::DataType;

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = schema.to_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

let mut writer = Ipc::new(Buffer::new()).with_schema(schema.clone());
writer.write_batch_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch]))?;

let mut reader = Ipc::new(Buffer::from_bytes(writer.handle().as_slice().to_vec()));
assert!(!reader.opened());

// Opening derives the schema once; every later question is answered from the cache.
reader.open()?;
assert!(reader.opened());
assert_eq!(reader.schema()?, schema);

reader.close()?;
assert!(!reader.opened());
// Still usable afterwards: it simply derives the schema again.
assert_eq!(reader.schema()?, schema);
```

`Ipc` works without `open`, like every other handle in [io.md](io.md): each call
materializes what it needs. What `open` adds is a schema derived once instead of once per
question, and `opened` reports exactly that - whether a schema is cached. `close` drops
it. A write also refreshes the cache, since the batches just written are the schema.

Deriving a schema means decoding the stream's header, and behind a coding it means
decoding the stream. That is the cost `open` moves to a known point.

## Absence

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::ipc::Ipc;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // A resource that does not exist yet holds no batches; it is not a parse failure.
    let missing = Ipc::new(Buffer::new()).with_schema(schema.clone());
    assert_eq!(missing.read_batch_reader(None)?.count(), 0);
    // The declared schema is what the empty reader reports.
    assert_eq!(missing.read_batch_reader(None)?.schema().fields().len(), 1);

    // Opening an absent stream succeeds and caches nothing.
    let mut empty = Ipc::new(Buffer::new());
    empty.open()?;
    assert!(!empty.opened());

    // Writing no batches still writes the schema, so the stream exists and is readable.
    let mut written = Ipc::new(Buffer::new()).with_schema(schema.clone());
    written.write_batch_reader(arrow::batch_reader(
        schema.to_arrow_schema()?,
        std::iter::empty::<RecordBatch>(),
    ))?;
    assert!(!written.handle().is_empty());
    assert_eq!(written.read_batch_reader(None)?.count(), 0);
    assert_eq!(written.schema()?, schema);
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
    assert missing.read_arrow_batch_reader().read_all().num_rows == 0

    # Writing no batches still writes the schema, so the stream exists and reads.
    written = IOBase(root / "empty.arrows")
    written.write_arrow_batch_reader(pa.Table.from_batches([], schema=schema))
    assert written.size > 0
    assert written.read_arrow_batch_reader().read_all().num_rows == 0
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
    assert.equal(missing.readArrowBatchReader().toTable().numRows, 0)

    // Writing no batches still writes the schema, so the stream exists and reads.
    const schema = new arrow.Schema([new arrow.Field('id', new arrow.Int64(), true)])
    const written = new IOBase(path.join(root, 'empty.arrows'))
    written.writeArrowBatchReader(new arrow.Table(schema))
    assert.ok(written.size > 0)
    assert.equal(written.readArrowBatchReader().toTable().numRows, 0)
    assert.equal(written.readArrowField().name, 'row')

    fs.rmSync(root, { recursive: true, force: true })
    ```

Reading follows the laziness rule the storage layer sets in [io.md](io.md): a location
that holds nothing yields nothing. An empty handle with a declared schema reports that
schema, and an empty handle without one reports an empty Arrow schema, so a caller can
probe a stream without an existence check first.

An empty stream and a stream of zero batches are different things, and the difference is
visible in the bytes: the second one was written, carries its schema, and answers
`schema` from the stream itself.

Anything that is not a stream fails on the spot rather than being guessed at.

=== "Rust"

    ```rust
    use yggdryl::io::Buffer;
    use yggdryl::ipc::{self, IpcOptions};

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
        handle.read_arrow_batch_reader()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes(Buffer.from('definitely not an Arrow IPC stream'))
    handle.mediaType = MimeType.ARROW_STREAM

    assert.throws(() => handle.readArrowField(), /Arrow/)
    assert.throws(() => handle.readArrowBatchReader(), /Arrow/)
    ```

The other record encoding in this build is [parquet.md](parquet.md), behind the non-default
`parquet` feature. It has the same `read_field`, `read_batch_reader`, and
`write_batch_reader` shape over the same shared settings, and adds the three a file
format needs that a stream does not. [`generic::Media`](generic.md) holds either one
without naming which.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/ipc-rust.ipynb){ download },
[Python](notebooks/ipc-python.ipynb){ download },
[JavaScript](notebooks/ipc-javascript.ipynb){ download }.

<!-- /notebooks -->
