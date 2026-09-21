# Arrow IPC

`yggdryl::ipc` reads and writes Arrow IPC streams over any byte handle.

## Contract

| Key | Value |
| --- | --- |
| Owns | `ipc::read_field`, `ipc::read_batch_reader`, `ipc::overwrite_arrow_reader`, `Ipc<H>`, `IpcOptions` |
| Handle surface | `overwrite_*`, `append_*`, keyed `merge_*`, `read_arrow_reader`, `read_arrow_field` from [`IOMedia`](../../holder/iobase/records.md) |
| Surfaces | native rows through `*_records` and Arrow batches through `*_arrow_reader`; one stream answers both |
| Merge | `merge_by` supplies identity only; the method name carries intent, never the key |
| Schema | self-describing; `options.field` declared skips the handle; root name defaults to `DEFAULT_ROOT_NAME` (`"row"`) |
| Pushdown | `field`, a non-null struct root naming a subset, projects at decode; keeps stored order and types, never casts |
| Coding | the content coding the name declares (`.gz`, `.zst`); `level` is the only compression setting |
| Cached | `open` caches schema and dimensions until `close`; writes and every `Ipc` builder drop the cache |
| Format settings | none beyond the shared [`IORecordOptions`](../options.md) fields |
| Errors | bytes that are not a stream fail `read_field` and `read_batch_reader` at once |
| Bindings | Rust: free functions and `Ipc<H>`; Python: `yggdryl.media.Ipc`, the class an `.arrows` handle answers, with `pyarrow.RecordBatchReader` over Arrow C Stream; JavaScript: `IOBase` with Arrow JS over the copied IPC bytes |

## Pages

The encoding lives in free functions - `ipc::read_field`, `ipc::read_batch_reader`, `ipc::overwrite_arrow_reader` - that take any handle and one `IpcOptions`; [`Ipc<H>`](options.md) is the stateful form that owns both and caches what `open` filled. Streaming is the only shape: a read returns a [`BatchReader`](../../arrow/readers.md) and a write consumes one, never a collected vector.

Two surfaces answer the same stream - native rows, with no Arrow type in the call, and Arrow batches - and each page takes both, split by direction, so one page answers how a stream is read and one how it is written.

| Page | Owns |
| --- | --- |
| [Read](read.md) | rows as native scalars, rows as Arrow batches, the schema alone |
| [Write](write.md) | overwrite, append and keyed merge, as native rows and as batches |
| [Options](options.md) | `Ipc<H>`, `IpcOptions`, the coding the name declares, absence |
| [Pushdown](pushdown.md) | column projection at decode, and the schema the stream carries |

[Parquet](../parquet/index.md) has the same `read_field`, `read_batch_reader`, and `overwrite_arrow_reader` shape behind the non-default `parquet` feature.

## Use

A name that says Arrow IPC is the whole configuration: the stream carries its own schema, and `open` caches the dimensions until `close`. `row_size` counts IPC message metadata and skips dictionary and record-batch bodies; `column_size` reads the canonical Struct field. Both describe the whole stream and ignore selection, partition filters, and read limits.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::holder::Holder;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
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

## Edges

- `read_scalar` or `write_scalar` on an `.arrows` name -> refused; a stream is records, not one structured document.
- `open` -> caches the schema and the dimensions; any write and every `Ipc` builder drop that cache.
- closed handle -> every call reads fresh metadata; construction already composed the wrapper, so `open` only caches.
- coding, absence, and unreadable bytes -> [Options](options.md#edges); projection and root naming -> [Pushdown](pushdown.md#edges).

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
