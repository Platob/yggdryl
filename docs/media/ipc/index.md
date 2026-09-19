# Arrow IPC

`yggdryl::ipc` reads and writes Arrow IPC streams over any byte handle.

## Contract

| Key | Value |
| --- | --- |
| Owns | `ipc::read_field`, `ipc::read_batch_reader`, `ipc::overwrite_arrow_reader`, `Ipc<H>`, `IpcOptions` |
| Handle surface | `overwrite_*`, `append_*`, keyed `merge_*`, `read_arrow_reader`, `read_arrow_field` from [`IOMedia`](../../holder/iobase/records.md) |
| Merge | `merge_by` supplies identity only; the method name carries intent, never the key |
| Schema | self-describing; `dtype` set skips the handle; root name defaults to `DEFAULT_ROOT_NAME` (`"row"`) |
| Pushdown | `field`, a non-null struct root naming a subset, projects at decode; keeps stored order and types, never casts |
| Coding | the content coding the name declares (`.gz`, `.zst`); `level` is the only compression setting |
| Cached | `open` caches schema and dimensions until `close`; writes and every `Ipc` builder drop the cache |
| Format settings | none beyond the shared [`IORecordOptions`](../options.md) fields |
| Errors | bytes that are not a stream fail `read_field` and `read_batch_reader` at once |
| Bindings | Rust: free functions and `Ipc<H>`; Python: `yggdryl.media.Ipc`, the class an `.arrows` handle answers, with `pyarrow.RecordBatchReader` over Arrow C Stream; JavaScript: `IOBase` with Arrow JS over the copied IPC bytes |

## Surfaces

| Page | Owns |
| --- | --- |
| [Scalars](scalar.md) | native rows in and out: `overwrite_records`, `append_records`, `merge_records`, `read_records` |
| [Arrow](arrow.md) | batch readers, the three write intents, column pushdown, schema carriage |

## Use

A name that says Arrow IPC is the whole configuration: the stream carries its own schema, and `open` caches the dimensions until `close`. `row_size` counts IPC message metadata and skips dictionary and record-batch bodies; `column_size` reads the canonical Struct field. Both describe the whole stream and ignore selection, partition filters, and read limits.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::holder::Holder;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType, StructureType};

    let field = DataType::from(StructureType::from_fields([DataType::Int64.required_field("id")])?)
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

## One stream, one configuration

Rust only.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::{IOBase, IOMedia, StructureType};
use yggdryl::holder::Buffer;
use yggdryl::ipc::Ipc;
use yggdryl::{DataType, Url};

let schema = DataType::from(StructureType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");
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

`Ipc<H>` implements `IOBase` by delegating to the handle it owns, which is why `read_range_bytes` works on it and why [`Media::Ipc`](../index.md) can hold it.

| `Ipc<H>` call | Effect |
| --- | --- |
| `record_options` | the defaults as the `RecordOptions` every canonical `IOMedia` call accepts |
| `handle`, `handle_mut`, `into_handle` | reach the wrapped handle |
| `options`, `options_mut` | change future defaults |
| `with_options` | replaces the whole settings value |
| `with_field`, `with_name` | reach the declared root |
| any `with_*`, `with_level` included | drops the opened metadata cache |

## Content coding comes from the name

The encoding applies the content coding the name declares on write and strips it on read: `trades.arrows.gz` round-trips through [gzip](../../coding/gzip.md), `trades.arrows.zst` through [zstd](../../coding/zstd.md), with identical calls, and `level` is the one compression setting.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::ipc::Ipc;
    use yggdryl::{DataType, Level, StructureType, Url};

    let schema = DataType::from(StructureType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from((0..512).collect::<Vec<i64>>()))],
    )?;

    let mut stored = Vec::new();
    for name in ["trades.arrows", "trades.arrows.gz", "trades.arrows.zst"] {
        let url = Url::from_str(&format!("file:///{name}"))?;
        // A handle whose name declares no coding ignores the level.
        let mut media = Ipc::new(Buffer::new().with_media_type(url.media_type()))
            .with_field(schema.clone())
            .with_level(Level::BEST);
        let options = media.record_options()?;
        media.overwrite_arrow_reader(
            arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]),
            &options,
        )?;

        // Identical calls on both sides, whatever the coding is.
        assert_eq!(media.read_arrow_reader(&options)?.count(), 1, "{name}");
        stored.push(media.handle().as_slice().to_vec());
    }

    // The bytes underneath are framed by the coding the name declared, and
    // each coded member is smaller than the stream it encodes.
    assert_eq!(&stored[1][..2], &[0x1F, 0x8B]);
    assert_eq!(&stored[2][..4], &[0x28, 0xB5, 0x2F, 0xFD]);
    assert!(stored[1].len() < stored[0].len() && stored[2].len() < stored[0].len());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.holder import Path

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    batch = pa.record_batch({"id": list(range(512))}, schema=schema)
    root = pathlib.Path(tempfile.mkdtemp())

    stored = []
    for name in ("trades.arrows", "trades.arrows.gz", "trades.arrows.zst"):
        handle = IOBase(root / name)
        # A handle whose name declares no coding ignores the level.
        options = handle.record_options()
        options.level = 9
        handle.overwrite_arrow_batch(batch, options=options)

        # Identical calls on both sides, whatever the coding is.
        assert handle.read_arrow_reader().read_all().num_rows == 512, name
        # The handle presents the decoded stream, so its bytes are the stream.
        assert handle.read_bytes()[:4] == bytes.fromhex("ffffffff"), name
        # Path addresses the stored bytes instead, coding and all.
        stored.append(Path(root / name).read_bytes())

    # The bytes underneath are framed by the coding the name declared, and
    # each coded member is smaller than the stream it encodes.
    assert stored[1][:2] == bytes.fromhex("1f8b")
    assert stored[2][:4] == bytes.fromhex("28b52ffd")
    assert len(stored[1]) < len(stored[0]) and len(stored[2]) < len(stored[0])
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
    const ids = Array.from({ length: 512 }, (_, index) => BigInt(index))
    const written = []
    for (const name of ['trades.arrows', 'trades.arrows.gz', 'trades.arrows.zst']) {
      const handle = new IOBase(path.join(root, name))
      // A handle whose name declares no coding ignores the level.
      handle.overwriteArrowTable(
        new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
        handle.recordOptions().withLevel(9),
      )

      // Identical calls on both sides, whatever the coding is.
      assert.equal(handle.readArrowReader().intoTable().numRows, 512, name)
      written.push(handle.readBytes())
    }

    // The bytes underneath are framed by the coding the name declared, and
    // each coded member is smaller than the stream it encodes.
    assert.deepEqual([...written[1].subarray(0, 2)], [0x1f, 0x8b])
    assert.deepEqual([...written[2].subarray(0, 4)], [0x28, 0xb5, 0x2f, 0xfd])
    assert.ok(written[1].length < written[0].length && written[2].length < written[0].length)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Options

IPC adds no setting of its own. `IpcOptions` holds the shared record settings as public fields - `name`, `field`, `filter`, `select`, `merge_by`, `safe`, `batch_row_size`, `batch_byte_size`, `max_row_size`, `max_byte_size`, `commit_row_size`, and `level` - and converts into [`RecordOptions`](../options.md#use), whose page demonstrates the settings each binding carries (`batch_byte_size` is Rust only). The `ipc::*` functions handle only the encoding seam; the [`IOMedia`](../../holder/iobase/records.md) path adds casting, re-chunking, the plan's `where` and `select`, limits, commit cadence, and write intent.

## Absence

A location that holds nothing yields nothing, the laziness rule [Bytes](../../holder/iobase/bytes.md) sets. Anything that is not a stream fails on the spot.

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia, StructureType};
    use yggdryl::holder::Buffer;
    use yggdryl::ipc::{self, Ipc, IpcOptions};
    use yggdryl::DataType;

    let schema = DataType::from(StructureType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");

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

    // Bytes that are not a stream fail at once.
    let garbage = Buffer::from_bytes(b"definitely not an Arrow IPC stream".to_vec());
    assert!(ipc::read_field(&garbage, &IpcOptions::new()).is_err());
    assert!(ipc::read_batch_reader(&garbage, None, &IpcOptions::new()).is_err());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pytest

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

    # Bytes that are not a stream fail at once.
    garbage = IOBase.from_bytes(b"definitely not an Arrow IPC stream")
    garbage.media_type = "application/vnd.apache.arrow.stream"
    with pytest.raises(ValueError):
        garbage.read_arrow_field()
    with pytest.raises(ValueError):
        garbage.read_arrow_reader()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase, MimeType } = require('yggdryl')

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

    // Bytes that are not a stream fail at once.
    const garbage = IOBase.fromBytes(Buffer.from('definitely not an Arrow IPC stream'))
    garbage.mediaType = MimeType.ARROW_STREAM
    assert.throws(() => garbage.readArrowField(), /Arrow/)
    assert.throws(() => garbage.readArrowReader(), /Arrow/)

    fs.rmSync(root, { recursive: true, force: true })
    ```

[Parquet](../parquet/index.md) has the same `read_field`, `read_batch_reader`, and `overwrite_arrow_reader` shape behind the non-default `parquet` feature.

## Edges

- `IpcOptions::dtype` set -> `read_field` builds the field without touching the handle.
- `field` accessor -> built from `name`, `dtype`, and `metadata` by [`IORecordOptions`](../options.md).
- handle with no content coding -> `level` does nothing.
- reading the bytes of a coded handle -> the decoded stream; `holder.Path` and `holder.File` address the stored, coded bytes.
- missing resource -> zero batches, not a parse failure; the reader reports the declared schema, or an empty Arrow schema without one.
- `open` on an absent stream -> succeeds and caches explicit zero dimensions.
- zero batches written -> the schema is still written; the stream exists and answers its schema from the bytes.
- bytes that are not a stream -> `read_field` and `read_batch_reader` error; Python raises `ValueError`, JavaScript throws matching `/Arrow/`.
- closed handle -> every call reads fresh metadata; construction already composed the wrapper, so `open` only caches.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib ipc::tests
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
