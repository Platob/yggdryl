# Apache Parquet

Read and write Apache Parquet over any handle; the footer's contents and the stateful `Parquet<H>` wrapper live on [Parquet footer](footer.md).

## Contract

| | |
| --- | --- |
| Owns | `yggdryl::parquet`: `ParquetOptions` and the free seams `read_arrow_schema`, `read_field`, `read_batch_reader`, `overwrite_arrow_reader`, `read_statistics`, taking the handle and a `&ParquetOptions` explicitly (Rust only) |
| Feature flag | `parquet`, non-default; without it the module is absent and [`RecordOptions::for_mime_type`](../options.md) reports `application/vnd.apache.parquet` as not implemented |
| Writes | `overwrite_arrow_reader`, `append_arrow_reader`, `merge_arrow_reader` under the [canonical signatures](../../holder/iobase/records.md); the media type selects Parquet, `merge_by` supplies row-identity keys only |
| Reads | `read_arrow_reader` returns an [`arrow::BatchReader`](../../arrow/readers.md), `read_arrow_field` the canonical non-null struct root [`Field`](../../types/field.md); `read_arrow_schema` and `read_statistics` are Parquet-specific |
| Pushdown | the read `field` is a `ProjectionMask` over root columns; excluded chunks are never located, decompressed, or decoded |
| Options | `compression` (default Zstandard, default level), `max_row_group_size` (default 1,048,576), `key_value_metadata`, plus the shared [`IORecordOptions`](../options.md) fields; `level` does nothing |
| Dimensions | `row_size` and `column_size` range-read only the eight-byte tail and footer; whole-file counts that ignore selection, filters, and limits |
| Cached | `open` retains the inferred wrapper and footer until `close`; writes invalidate it; closed calls read a fresh footer |
| Coded handles | any coding other than identity is refused on reads and writes before anything is encoded |
| Bindings | Python exchanges `pyarrow.RecordBatchReader` over the Arrow C Stream; JavaScript exchanges Arrow JS values over copied IPC, one batch per stream; neither builds a table unasked |

## Surfaces

| Page | Owns |
| --- | --- |
| [Scalars](scalar.md) | native rows in and out: `overwrite_records`, `append_records`, `merge_records`, `read_records` |
| [Arrow](arrow.md) | batch readers, the three write intents, the projection mask |
| [Footer](footer.md) | footer metadata, statistics, field ids, the caching `Parquet<H>` wrapper |

## Use

The handle's media type selects Parquet, so no call names a format. `row_size` and `column_size` range-read the eight-byte tail and the footer, never a column chunk: fresh calls read a footer each time; an opened handle answers from its cached one until `close`.

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
    let mut handle = Holder::buffer(Buffer::new().with_media_type(MimeType::PARQUET.into()));
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

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "dimensions.parquet")
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
    const handle = new IOBase(path.join(root, 'dimensions.parquet'))
    handle.overwriteArrowTable(arrow.tableFromArrays({ id: [1, 2] }))
    handle.open()
    assert.deepEqual([handle.rowSize, handle.columnSize], [2, 1])
    assert.equal(handle.readArrowField().name, 'row')
    handle.close()
    fs.rmSync(root, { recursive: true, force: true })
    ```

## Options

Parquet's own settings and the shared ones are flat fields of one value.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, Level, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from((0..1_000).collect::<Vec<i64>>()))],
    )?;

    // Parquet's own settings and the shared ones are flat fields on one struct.
    let options = ParquetOptions::new()
        .with_max_row_group_size(4_096)
        .with_key_value("iceberg.schema-id", "7")
        .with_batch_row_size(256)
        .with_name("trade");

    assert_eq!(options.max_row_group_size, 4_096);
    assert_eq!(
        options.key_value_metadata,
        [("iceberg.schema-id".to_owned(), "7".to_owned())]
    );
    assert_eq!(options.batch_row_size(), Some(256));
    assert_eq!(options.name(), "trade");
    assert!(!options.safe());

    // Unused here: Parquet compresses pages itself.
    assert_eq!(options.level, Level::DEFAULT);

    let mut media =
        Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into())).with_options(options);
    let call_options = media.record_options()?;
    media.overwrite_arrow_reader(
        arrow::batch_reader(arrow_schema, [batch]),
        &call_options,
    )?;

    // batch_row_size bounds the reader, so no batch holds all 1,000 rows.
    let rows: Vec<usize> = media
        .read_arrow_reader(&call_options)?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .map(arrow_array::RecordBatch::num_rows)
        .collect();
    assert_eq!(rows.iter().sum::<usize>(), 1_000);
    assert!(rows.iter().all(|count| *count <= 256), "{rows:?}");

    // The root name names the Field recovered from the footer.
    assert_eq!(media.read_arrow_field(&call_options)?.name(), "trade");

    // A declared schema is returned as-is, so an empty handle answers without a footer.
    let declared =
        Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into())).with_field(field.clone());
    let declared_options = declared.record_options()?;
    assert_eq!(declared.read_arrow_field(&declared_options)?, field);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    batch = pa.record_batch({"id": list(range(1_000))}, schema=schema)

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")

    # Parquet's own settings and the shared ones are properties of one value.
    options = handle.record_options()
    options.max_row_group_size = 4_096
    options.key_value_metadata = {"iceberg.schema-id": "7"}
    options.batch_row_size = 256
    options.name = "trade"

    assert options.max_row_group_size == 4_096
    assert options.key_value_metadata == {"iceberg.schema-id": "7"}
    assert options.batch_row_size == 256
    assert options.name == "trade"
    assert not options.safe

    handle.overwrite_arrow_batch(batch, options=options)

    # batch_row_size bounds the reader, so no batch holds all 1,000 rows.
    counts = [part.num_rows for part in handle.read_arrow_reader(options=options)]
    assert sum(counts) == 1_000
    assert all(count <= 256 for count in counts), counts

    # The root name names the Field recovered from the footer.
    assert handle.read_arrow_field(options=options).name == "trade"
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
    const handle = new IOBase(path.join(root, 'trades.parquet'))

    // Parquet's own settings and the shared ones are properties of one value.
    const options = handle
      .recordOptions()
      .withMaxRowGroupSize(4_096)
      .withKeyValue('iceberg.schema-id', '7')
      .withBatchRowSize(256)
      .withName('trade')

    assert.equal(options.maxRowGroupSize, 4_096)
    assert.deepEqual(options.keyValueMetadata, [{ key: 'iceberg.schema-id', value: '7' }])
    assert.equal(options.batchRowSize, 256)
    assert.equal(options.name, 'trade')
    assert.equal(options.safe, false)

    const ids = Array.from({ length: 1_000 }, (_, index) => BigInt(index))
    handle.overwriteArrowTable(
      new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
      options,
    )

    // batchRowSize bounds the reader, so no batch holds all 1,000 rows.
    const counts = [...handle.readArrowReader(options)].map((batch) => batch.numRows)
    assert.equal(counts.reduce((total, count) => total + count, 0), 1_000)
    assert.ok(counts.every((count) => count <= 256), counts.join())

    // The root name names the Field recovered from the footer.
    assert.equal(handle.readArrowField(options).name, 'trade')

    fs.rmSync(root, { recursive: true, force: true })
    ```

| setting | effect |
| --- | --- |
| `compression` | codec applied to pages inside the file |
| `max_row_group_size` | row bound that decides how many row groups the file gets |
| `key_value_metadata` | footer entries next to the ones the writer adds itself |
| `level` | nothing; Parquet has no outer coding to apply it to |
| shared | `name`, `field`, `filter`, `selector`, `merge_by`, `safe`, `batch_row_size`, `batch_byte_size`, `max_row_size`, `max_byte_size`, `commit_row_size` |
| `Parquet::with_options` | replaces the whole set |
| `with_field`, `with_name` | reach through to the declared root; `name` roots a declared field and one recovered from the footer alike |

## Compression

The bindings name page compression as the text the `parquet` crate parses: `zstd(3)`, `snappy`, `uncompressed`. Compression is a write setting only; the footer records the codec, so every runtime reads every file. A coding around the whole file would move the footer out of reach, so a coded name is refused before anything is encoded, and the handle is left untouched.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use parquet::basic::Compression;
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, MimeType, Url};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?
    .required_field("row");

    let ids: Vec<i64> = (0..1_024).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    let arrow_schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )?;

    let mut sizes = Vec::new();
    for compression in [
        Compression::UNCOMPRESSED,
        Compression::SNAPPY,
        Compression::ZSTD(Default::default()),
    ] {
        // One batch per read, so the comparison is not split by the default bound.
        let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
            .with_options(
                ParquetOptions::new()
                    .with_compression(compression)
                    .with_batch_row_size(batch.num_rows()),
            );
        let options = media.record_options()?;
        media.overwrite_arrow_reader(
            arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]),
            &options,
        )?;

        // Nothing on the read side names the compression: the footer records it.
        let read = media
            .read_arrow_reader(&options)?
            .collect::<Result<Vec<_>, _>>()?;
        assert_eq!(read, [batch.clone()], "{compression:?}");
        sizes.push(media.handle().size());
    }

    assert!(sizes[0] > sizes[1] && sizes[0] > sizes[2], "{sizes:?}");

    // A coding around the whole file is refused, and nothing is published.
    let coded = Url::from_str("file:///trades.parquet.gz")?;
    let mut media = Parquet::new(Buffer::new().with_media_type(coded.media_type()));
    let options = media.record_options()?;
    let message = media
        .overwrite_arrow_reader(
            arrow::batch_reader(Arc::clone(&arrow_schema), [batch]),
            &options,
        )
        .unwrap_err()
        .to_string();

    assert!(message.contains("parquet compresses"), "{message}");
    assert!(message.contains("ParquetOptions::compression"), "{message}");
    assert!(media.handle().is_empty());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    rows = 1_024
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    table = pa.table(
        {"id": list(range(rows)), "symbol": ["AAPL"] * rows}, schema=schema
    )

    sizes = []
    for compression in ("uncompressed", "snappy", "zstd(1)"):
        handle = IOBase(root / f"trades-{compression}.parquet")
        # One batch per read, so the comparison is not split by the default bound.
        options = handle.record_options()
        options.compression = compression
        options.batch_row_size = rows
        handle.overwrite_arrow_table(table, options=options)

        # Nothing on the read side names the compression: the footer records it.
        read = handle.read_arrow_reader(options=options).read_all()
        assert read.num_rows == rows, compression
        sizes.append(handle.size)

    assert sizes[0] > sizes[1] and sizes[0] > sizes[2], sizes

    # A coding around the whole file is refused, and nothing is published.
    coded = IOBase(root / "trades.parquet.gz")
    with pytest.raises(ValueError, match="parquet compresses"):
        coded.overwrite_arrow_table(table)
    assert coded.size == 0
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
    const ids = Array.from({ length: 1_024 }, (_, index) => BigInt(index))
    const table = new arrow.Table({
      id: arrow.vectorFromArray(ids, new arrow.Int64()),
      symbol: arrow.vectorFromArray(ids.map(() => 'AAPL'), new arrow.Utf8()),
    })

    const sizes = []
    for (const compression of ['uncompressed', 'snappy', 'zstd(1)']) {
      const handle = new IOBase(path.join(root, `trades-${compression}.parquet`))
      // One batch per read, so the comparison is not split by the default bound.
      const options = handle
        .recordOptions()
        .withCompression(compression)
        .withBatchRowSize(table.numRows)
      handle.overwriteArrowTable(table, options)

      // Nothing on the read side names the compression: the footer records it.
      const read = handle.readArrowReader(options).intoTable()
      assert.equal(read.numRows, 1_024, compression)
      sizes.push(handle.size)
    }

    assert.ok(sizes[0] > sizes[1] && sizes[0] > sizes[2], sizes.join())

    // A coding around the whole file is refused, and nothing is published.
    const coded = new IOBase(path.join(root, 'trades.parquet.gz'))
    assert.throws(() => coded.overwriteArrowTable(table), /parquet compresses/)
    assert.equal(coded.size, 0)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Edges

- `trades.parquet.gz`, or any non-identity coding -> refused on reads and writes with `parquet compresses`, naming `ParquetOptions::compression`.
- other encodings, such as [Arrow IPC](../ipc/index.md), take a coded name through the handle's [coding](../../coding/index.md); Parquet alone refuses one.
- `level` -> ignored; `compression` decides how the file compresses.
- declared `dtype` -> `read_arrow_field` returns it without reading the file, so an empty handle answers without a footer.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib parquet::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/parquet/read_rows
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- 'io_dimensions/parquet/(row_size|column_size|read_arrow_field)'
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/parquet
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/parquet
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_parquet.py
    python/.venv/bin/python python/benchmarks/media.py --filter "parquet write" --filter "parquet read whole" --filter "parquet read subset" --filter "parquet read records" --filter "parquet row size" --filter "parquet column size" --filter "PyArrow parquet"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    YGGDRYL_BENCH_FILTER=records/read_parquet_into_ipc npm run --prefix node bench:media
    YGGDRYL_BENCH_FILTER=records/read_parquet_pushdown npm run --prefix node bench:media
    ```

## Performance

### Batch operations

Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23).

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 18.6 ms | 3.52M rows/s |
| `overwrite_arrow_reader` | 4,096 | 3.91 ms | 1.05M rows/s |
| `append_arrow_reader` | 4,096 | 8.06 ms | 508k rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 9.03 ms | 453k rows/s |

The read fixture holds 65,536 rows and four columns, the write fixture 4,096 rows. Criterion prepares the stored side for append and keyed merge (the upsert) outside the timer.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/parquet/read_rows
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/parquet
```

### Dimensions

Same 65,536-row fixture and host; fresh calls read the footer, opened calls answer from the cache.

| operation | fresh | opened |
| --- | ---: | ---: |
| `row_size` | 8.95 us | 9.12 ns |
| `column_size` | 18.1 us | 6.86 ns |
| `read_arrow_field` | 297 us | 119 us |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- 'io_dimensions/parquet/(row_size|column_size|read_arrow_field)'
```

### Against PyArrow

One containerized x86_64 Linux run of `python/benchmarks/media.py` with `--min-time 0.1 --repeat 3`: 65,536 rows, 4 columns, 8 batches.

```text
parquet write reader             6.932 ms    9.5M rows/s
PyArrow parquet write baseline   6.495 ms   10.1M rows/s
parquet read whole               2.620 ms   25.0M rows/s
PyArrow parquet read baseline    2.195 ms   29.9M rows/s
```

Both directions sit within ~15% of PyArrow because both sides drive the same `parquet` machinery; [Arrow IPC](../ipc/index.md) carries that encoding's rows from the same run.

```bash
python/.venv/bin/python python/benchmarks/media.py --filter "parquet write" --filter "parquet read whole" --filter "parquet read subset" --filter "parquet read records" --filter "parquet row size" --filter "parquet column size" --filter "PyArrow parquet"
```
