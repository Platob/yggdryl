# Arrow IPC options

What an IPC read or write is configured with: the shared record settings, the coding the name declares, and the stateful `Ipc<H>` that carries both.

## Contract

| Key | Value |
| --- | --- |
| Owns | `IpcOptions`, `Ipc<H>` |
| Format settings | none beyond the shared [`IORecordOptions`](../options.md) fields |
| Coding | the content coding the name declares (`.gz`, `.zst`); `level` is the only compression setting |
| Cached | `open` caches schema and dimensions until `close`; writes and every `Ipc` builder drop the cache |
| Delegation | `Ipc<H>` implements [`IOBase`](../../holder/iobase/bytes.md) through the handle it owns |
| Absence | a location that holds nothing yields nothing; bytes that are not a stream fail on the spot |

## Use

One stream, one configuration: `Ipc<H>` owns the handle and the options together, so no call repeats the schema, the root name, or the coding. Rust only - Python and JavaScript make the same calls on the handle itself, as on the [IPC overview](index.md#use).

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::{IOBase, IOMedia, StructType};
use yggdryl::holder::Buffer;
use yggdryl::ipc::Ipc;
use yggdryl::{DataType, Url};

let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");
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
    use yggdryl::{DataType, Level, StructType, Url};

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");
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

## Settings

IPC adds no setting of its own. `IpcOptions` holds the shared record settings as public fields - `name`, `field`, `filter`, `select`, `merge_by`, `safe`, `batch_row_size`, `batch_byte_size`, `max_row_size`, `max_byte_size`, `commit_row_size`, and `level` - and converts into [`RecordOptions`](../options.md#use), whose page demonstrates the settings each binding carries (`batch_byte_size` is Rust only). The `ipc::*` functions handle only the encoding seam; the [`IOMedia`](../../holder/iobase/records.md) path adds casting, re-chunking, the plan's `where` and `select`, limits, commit cadence, and write intent.

## Absence

A location that holds nothing yields nothing, the laziness rule [Bytes](../../holder/iobase/bytes.md) sets. Anything that is not a stream fails on the spot.

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia, StructType};
    use yggdryl::holder::Buffer;
    use yggdryl::ipc::{self, Ipc, IpcOptions};
    use yggdryl::DataType;

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");

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

## Edges

- `IpcOptions::field` declared -> `read_field` answers it without touching the handle.
- `field` accessor -> the declared root, cloned by [`IORecordOptions`](../options.md); declaring one sets `name`, and setting `name` renames it, so the two never disagree.
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
    cargo test --features "iceberg internals parquet" -p yggdryl --test ipc -- mod_::internal
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/ipc
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/ipc
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_init.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/records.test.js
    ```
