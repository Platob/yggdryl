# Media

A handle's name picks the encoding, the compression and the charset; the read and write calls never change.

| Section | Declared by | Build |
| --- | --- | --- |
| [Arrow IPC](#arrow-ipc) | `application/vnd.apache.arrow.stream`, `.arrows` | default |
| [Parquet](#parquet) | `application/vnd.apache.parquet`, `.parquet` | `parquet` feature |
| [Avro](#avro) | `application/avro`, `.avro` | default |
| [Plain text](#plain-text) | `text/plain`, `.txt`, `.log` | default |
| [JSON](#json) | `application/json`, `application/x-ndjson`, `.json`, `.jsonl` | default |
| [YAML](#yaml) | `application/yaml`, `.yaml` | default |
| [TOML](#toml) | `application/toml`, `.toml` | default |
| [Iceberg](#iceberg) | a table folder | `iceberg` feature |
| [Compression](#compression) | `.gz`, `.zz`, `.zst` suffix | default |
| [Charsets](#charsets) | `;charset=` parameter | default |

## Read and write

Every record encoding answers the same calls through [`IOMedia`](../holder/index.md#records): `overwrite_records`, `append_records`, `merge_records`, `read_records` for native rows, and the `*_arrow_*` twins for Arrow batches.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};

    struct Quote(i64, &'static str);

    impl From<Quote> for Scalar {
        fn from(row: Quote) -> Self {
            Scalar::from_sequence([Scalar::from(row.0), Scalar::from(row.1)])
        }
    }

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("venue"),
    ])?)
    .required_field("row");
    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?.with_field(field.clone());

    handle.overwrite_records([Quote(1, "XNAS"), Quote(2, "XNYS")], &options)?;
    handle.append_records([Quote(3, "XLON")], &options)?;
    handle.merge_records([Quote(2, "XPAR")], &options.clone().with_merge_by(["id"])?)?;

    // Rust reads Arrow as a stream of record columns, one per batch, and a
    // record column lends each child column by name.
    let mut venues = Vec::new();
    for records in handle.read_arrow(Some(&options.clone().with_field(field.clone())))? {
        let venue = records?.child("venue").cloned().expect("a venue column");
        for row in 0..venue.len() {
            venues.push(venue.scalar(row)?);
        }
    }

    assert_eq!(venues.len(), 3);
    assert!(venues.contains(&Scalar::from("XPAR")) && !venues.contains(&Scalar::from("XNYS")));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")

    # Mappings are rows, and a non-empty source infers the field from them.
    handle.overwrite_records([{"id": 1, "venue": "XNAS"}, {"id": 2, "venue": "XNYS"}])
    handle.append_records([{"id": 3, "venue": "XLON"}])

    merging = handle.record_options()
    merging.merge_by = ["id"]
    handle.merge_records([{"id": 2, "venue": "XPAR"}], options=merging)

    # Plain dictionaries back, one row at a time.
    rows = list(handle.read_records())
    assert len(rows) == 3
    assert {row["venue"] for row in rows} == {"XNAS", "XPAR", "XLON"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.arrows'))

    // Plain objects are rows; the writers widen them into the stream.
    handle.overwriteRecords([{ id: 1n, venue: 'XNAS' }, { id: 2n, venue: 'XNYS' }])
    handle.appendRecords([{ id: 3n, venue: 'XLON' }])
    handle.mergeRecords(
      [{ id: 2n, venue: 'XPAR' }],
      handle.recordOptions().withMergeBy(['id']),
    )

    const rows = [...handle.readRecords()]
    assert.equal(rows.length, 3)
    assert.deepEqual(
      new Set(rows.map((row) => row.venue)),
      new Set(['XNAS', 'XPAR', 'XLON']),
    )

    fs.rmSync(root, { recursive: true, force: true })
    ```

### Arrow batches

A read returns an [`arrow::BatchReader`](../arrow/readers.md); only the current batch is alive.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    let schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS"), None])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    // Batches arrive one at a time; only the current one is alive.
    let mut rows = 0;
    for batch in handle.read_arrow_reader(&options)? {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 3);
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
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(
        pa.record_batch({"id": [1, 2, 3], "venue": ["XNAS", "XNYS", None]}, schema=schema)
    )

    # A pyarrow.RecordBatchReader over the Arrow C Stream; batches arrive one at a time.
    rows = sum(part.num_rows for part in handle.read_arrow_reader())
    assert rows == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowReader(
      BatchReader.from(
        new arrow.Table({
          id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
          venue: arrow.vectorFromArray(['XNAS', 'XNYS', null], new arrow.Utf8()),
        }),
      ),
    )

    // Batches arrive one at a time; only the current one is alive.
    let rows = 0
    for (const batch of handle.readArrowReader()) {
      rows += batch.numRows
    }
    assert.equal(rows, 3)
    ```

### Options

One `RecordOptions` drives every encoding: the root `field`, `select`, `filter`, `batch_row_size`, `merge_by`, `level`, plus the settings one encoding owns.

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{DataType, MimeType, StructType, Url};

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");

    let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?
        .with_field(schema.clone())
        .with_batch_row_size(1024);

    assert_eq!(options.mime_type(), MimeType::PARQUET);
    assert_eq!(options.field(), Some(schema.clone()));
    assert_eq!(options.name(), "row");
    assert!(options.select().is_all());
    assert!(options.filter().is_always_true());
    assert_eq!(options.batch_row_size(), Some(1024));
    assert_eq!(options.stable_hash(), options.clone().stable_hash());
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import RecordOptions

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    # The media type names the encoding, so there is no format argument.
    options = RecordOptions("trades.parquet")
    options.field = schema
    options.batch_row_size = 1024
    options.commit_row_size = 10_000

    assert str(options.mime_type) == "application/vnd.apache.parquet"
    assert options.name == "row"
    assert [child.name for child in options.field.dtype] == ["id"]
    assert options.select.is_all
    assert options.filter.is_always_true
    assert options.batch_row_size == 1024
    assert options.commit_row_size == 10_000

    # A setting one encoding has reads as None on an encoding that has none.
    assert options.max_row_group_size == 1_048_576
    stream = RecordOptions("trades.arrows")
    assert str(stream.mime_type) == "application/vnd.apache.arrow.stream"
    assert stream.max_row_group_size is None

    # `level` is shared, and applies where the handle declares a content coding.
    stream.level = 9
    assert stream.level == 9
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, RecordOptions, fields } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })

    const options = RecordOptions.from('trades.parquet')
      .withField(schema)
      .withBatchRowSize(1024)

    assert.equal(String(options.mimeType), 'application/vnd.apache.parquet')
    assert.equal(options.name, 'row')
    assert.ok(options.field.equals(schema))
    assert.ok(options.select.isAll)
    assert.ok(options.filter.isAlwaysTrue)
    assert.equal(options.batchRowSize, 1024)

    // A setting one encoding has reads as null on an encoding that has none.
    assert.equal(options.maxRowGroupSize, 1_048_576)
    const stream = RecordOptions.from('trades.arrows')
    assert.equal(stream.mimeType.toString(), 'application/vnd.apache.arrow.stream')
    assert.equal(stream.maxRowGroupSize, null)

    // `level` is shared, and applies where the handle declares a content coding.
    stream.level = 9
    assert.equal(stream.level, 9)

    // `with*` returns a new value rather than changing the one it was built from.
    assert.equal(options.withSafe(true).safe, true)
    assert.equal(options.safe, false)
    ```

## Arrow IPC

The stream carries its schema. A narrower `field` is a column pushdown: skipped columns are never decoded.

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

### Arrow IPC performance

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

Through the `Media` enum, which redirects to the same implementation:

| operation through `Media::Ipc` | estimate | throughput |
| --- | ---: | ---: |
| overwrite | 82.2 us | 49.8M rows/s |
| append | 424 us | 9.67M rows/s |
| keyed merge (upsert) | 6.41 ms | 639k rows/s |

Criterion prepares the stored side for append and merge outside the timer. Sub-millisecond estimates are regression anchors.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/media_ipc
```

## Parquet

Pages are compressed inside the file (`compression`), and the footer records the codec, so reads name nothing. A coded name such as `.parquet.gz` is refused.

A read's `filter` skips every row group whose footer statistics rule it out before a page is decoded, and the rows of the groups that remain are filtered as always. A column's statistics count only when it is stored as the type the filter reads and the read restores no default into its nulls; a floating-point column's minimum and maximum never count, because writers leave NaN out of them, though its null count does. A file of up to a megabyte is read in one request, unless a `max_row_size` without a filter lets its footer be read first; a larger one always has its footer read first. After a footer-first read only the column chunks of the row groups and columns the read keeps are fetched, and chunks less than a megabyte apart come in one request, so a small pruned group or unprojected column between kept ones is read with them. Either way the bytes are copied into memory the reader owns, so rewriting the file while a reader or its batches live is safe. A read yields 65,536-row batches unless `batch_row_size` bounds them; without a filter, never more than `max_row_size` asks for, and under any `max_row_size` - of one file, a folder of them, or an Iceberg table - it decodes lazily, one file at a time on one thread. From a megabyte of column chunks up, it decodes row groups side by side - and, with fewer row groups than threads, each row group's columns - handing batches back in file order, each thread at most sixteen batches ahead of the consumer; split across threads, no batch spans two row groups. A write feeds each row group's column writers as its input arrives - at once on one thread, in feeds of 32 MiB of Arrow input on several, their columns side by side - and the file is byte for byte the one a single thread writes.

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
    use yggdryl::{DataType, MimeType, StructType, Url};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
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

### Parquet performance

#### Batch operations

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

#### Dimensions

Same 65,536-row fixture and host; fresh calls read the footer, opened calls answer from the cache.

| operation | fresh | opened |
| --- | ---: | ---: |
| `row_size` | 8.95 us | 9.12 ns |
| `column_size` | 18.1 us | 6.86 ns |
| `read_arrow_field` | 297 us | 119 us |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- 'io_dimensions/parquet/(row_size|column_size|read_arrow_field)'
```

#### Against PyArrow

One containerized x86_64 Linux run of `python/benchmarks/media.py` with `--min-time 0.1 --repeat 3`: 65,536 rows, 4 columns, 8 batches; Intel Xeon @ 2.10 GHz, 4 cores, rustc 1.94.1, CPython 3.11.15, PyArrow 25.0.1, release wheel.

```text
parquet write reader             6.662 ms    9.8M rows/s
PyArrow parquet write baseline   8.768 ms    7.5M rows/s
parquet read whole               2.361 ms   27.8M rows/s
PyArrow parquet read baseline    2.500 ms   26.2M rows/s
```

On a file this small the read and the write are both ahead of PyArrow; [Streaming against PyArrow](#streaming-against-pyarrow) measures the sizes where the threads pay. [Arrow IPC](#arrow-ipc) carries that encoding's rows from the same run.

```bash
python/.venv/bin/python python/benchmarks/media.py --filter "parquet write" --filter "parquet read whole" --filter "parquet read subset" --filter "parquet read records" --filter "parquet row size" --filter "parquet column size" --filter "PyArrow parquet"
```

#### Streaming against PyArrow

One run of `python/benchmarks/media/parquet.py --repeat 7` on the same host, release wheel. Every read case is one file PyArrow wrote, read both ways: `pyarrow.parquet.read_table` and `ParquetFile.iter_batches(batch_size=65536)` against `read_arrow_reader` drained whole and streamed batch by batch. The filtered cases read one 4M-row Zstandard file of 32 row groups through a filter, `read_table(filters=...)` against a `filter` string: on the sorted key both skip the row groups the footer rules out, on the other columns every row group is read and its rows tested. Every write case is one table written by `pyarrow.parquet.write_table` and by `overwrite_arrow_table`, Zstandard level 1 on both sides. The ratios are PyArrow's best time over this crate's, so above one is in this crate's favor.

| read | `read_table` | `iter_batches` | whole | streamed | x whole | x streamed |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1M trade rows, 1 row group, Zstandard | 44.59 ms | 56.58 ms | 26.58 ms | 22.67 ms | 1.68 | 2.50 |
| 1M trade rows, 8 row groups, Zstandard | 35.40 ms | 54.93 ms | 26.95 ms | 27.07 ms | 1.31 | 2.03 |
| 1M trade rows, 1 row group, Snappy | 37.86 ms | 53.53 ms | 22.61 ms | 22.89 ms | 1.67 | 2.34 |
| 1M trade rows, uncompressed | 32.76 ms | 42.13 ms | 25.12 ms | 24.54 ms | 1.30 | 1.72 |
| 1M trade rows, 2 of 6 columns | 19.15 ms | 21.98 ms | 17.88 ms | 17.29 ms | 1.07 | 1.27 |
| 4M trade rows, 4 row groups, Zstandard | 118.93 ms | 214.65 ms | 91.84 ms | 80.78 ms | 1.29 | 2.66 |
| 1M string-heavy rows, Zstandard | 81.54 ms | 81.51 ms | 65.76 ms | 66.29 ms | 1.24 | 1.23 |
| 64K trade rows, Zstandard | 4.10 ms | 4.18 ms | 2.42 ms | 3.28 ms | 1.69 | 1.28 |

| filtered read, 4M rows in 32 row groups | `read_table(filters=...)` | `read_arrow_reader(filter=...)` | x |
| --- | ---: | ---: | ---: |
| `id < 100000`, 1 row group of 32 | 10.12 ms | 5.37 ms | 1.89 |
| `id between 2000000 and 2100000`, 2 row groups of 32 | 14.08 ms | 8.23 ms | 1.71 |
| `price > 990.0`, 1% of rows | 134.85 ms | 116.82 ms | 1.15 |
| `symbol = 'AAPL'`, 12.5% of rows | 136.02 ms | 119.14 ms | 1.14 |

| write | `write_table` | `overwrite_arrow_table` | x |
| --- | ---: | ---: | ---: |
| 1M trade rows | 189.81 ms | 100.73 ms | 1.88 |
| 4M trade rows | 800.21 ms | 333.11 ms | 2.40 |
| 1M string-heavy rows | 226.40 ms | 177.79 ms | 1.27 |
| 64K trade rows | 20.83 ms | 15.57 ms | 1.34 |

What the reader does with the time: batches of 65,536 rows rather than the Parquet crate's 1,024, so per-batch work - allocation, the Python crossing - is paid sixty-four times less; row groups, then columns, decoded on every thread; only the column chunks a read keeps fetched, and copied once. The writer encodes each row group's columns on every thread. The Python extension allocates through mimalloc, the allocator PyArrow ships, because the system allocator maps fresh pages for every decoded buffer: a 4M-row read took some 62,000 page faults before, and 800 after. The one close case is a two-column projection, where two columns are all the threads there are.

```bash
python/.venv/bin/python python/benchmarks/media/parquet.py --repeat 7
```

#### Footer statistics

Local release-build spot-check of the Python and JavaScript binding boundary; fixtures differ, so rows are per-runtime anchors, not a comparison.

| operation | runtime | rows | estimate |
| --- | --- | ---: | ---: |
| footer to native record (`read_parquet_statistics`) | Python | 65,536 | 504 us |
| footer to native record (`readParquetStatistics`) | JavaScript | 10,000 | 728 us |
| projected WKB to native record (`read_parquet_geospatial_statistics`) | Python | 8,192 | 2.64 ms |
| projected WKB to native record (`readParquetGeospatialStatistics`) | JavaScript | 10,000 | 3.26 ms |

```bash
python/.venv/bin/python python/benchmarks/media.py --filter "parquet read statistics" --filter "parquet read geospatial stats" --filter "parquet read arrow field"
YGGDRYL_BENCH_FILTER=records/read_parquet_statistics npm run --prefix node bench:media
YGGDRYL_BENCH_FILTER=records/read_parquet_geospatial_statistics npm run --prefix node bench:media
```

## Avro

A container carries its writer schema; a reader schema resolves renames, promotions and defaults.

=== "Rust"

    ```rust
    use yggdryl::avro::Schema;
    use yggdryl::holder::Buffer;
    use yggdryl::Scalar;
    use yggdryl::json;
    use yggdryl::avro;

    let writer = json::from_utf8(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"symbol","type":"string"},
            {"name":"qty","type":"int"},
            {"name":"venue","type":"string"}]}"#,
    )?;
    let reader = Schema::from_str(
        r#"{"type":"record","name":"trade","fields":[
            {"name":"quantity","aliases":["qty"],"type":"long"},
            {"name":"note","type":"string","default":"none"}]}"#,
    )?;
    let row = json::from_utf8(r#"{"symbol":"AAPL","qty":100,"venue":"XNAS"}"#)?;
    let mut handle = Buffer::new();
    avro::write_container(&mut handle, &writer, &[], &[row])?;

    let decoded = avro::read_container_resolved(&handle, &reader)?;
    assert_eq!(
        decoded.rows[0].get_key_str("quantity").and_then(Scalar::as_i64),
        Some(100),
    );
    assert_eq!(decoded.rows[0].len(), 2, "unwanted writer fields are skipped");
    ```

=== "Python"

    ```python
    from yggdryl import avro

    writer = {
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "symbol", "type": "string"},
            {"name": "qty", "type": "int"},
            {"name": "venue", "type": "string"},
        ],
    }
    reader = avro.Schema({
        "type": "record",
        "name": "trade",
        "fields": [
            {"name": "quantity", "aliases": ["qty"], "type": "long"},
            {"name": "note", "type": "string", "default": "none"},
        ],
    })
    encoded = avro.dumps([{"symbol": "AAPL", "qty": 100, "venue": "XNAS"}], writer)

    assert avro.loads(encoded, reader_schema=reader).rows == [
        {"note": "none", "quantity": 100}
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { avro } = require('yggdryl')

    const writer = {
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'symbol', type: 'string' },
        { name: 'qty', type: 'int' },
        { name: 'venue', type: 'string' },
      ],
    }
    const reader = new avro.Schema({
      type: 'record',
      name: 'trade',
      fields: [
        { name: 'quantity', aliases: ['qty'], type: 'long' },
        { name: 'note', type: 'string', default: 'none' },
      ],
    })
    const encoded = avro.dumps(
      [{ symbol: 'AAPL', qty: 100, venue: 'XNAS' }],
      writer,
    )

    assert.deepEqual(avro.loads(encoded, { readerSchema: reader }).rows, [
      { note: 'none', quantity: 100 },
    ])
    ```

### Avro performance

#### Against polars and fastavro

One run of `python/benchmarks/media/avro.py --repeat 5` on a containerized four-core x86_64 Linux host, release wheel: CPython 3.11.15, PyArrow 25.0.1, polars 1.44.2, fastavro 1.12.2. Every read case is one container fastavro wrote in blocks of about 64,000 bytes, the Java writer's default, read three ways: `polars.read_avro`, `fastavro.reader` drained row by row, and `read_arrow_reader(...).read_all()`. The key and price columns polars reads are checked against this crate's, value for value, before anything is timed. Every write case is one table written by `DataFrame.write_avro`, `fastavro.writer` and `overwrite_arrow_table` with the same block codec. The ratios are the other library's best time over this crate's, so above one is in this crate's favor. polars cannot read Zstandard blocks - it refuses them or misreads them - so that row has no polars column.

| read | polars | fastavro | yggdryl | x polars | x fastavro |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1M trade rows, null | 177.31 ms | 2,706.5 ms | 40.14 ms | 4.42 | 67.42 |
| 1M trade rows, deflate | 386.79 ms | 2,674.2 ms | 63.51 ms | 6.09 | 42.11 |
| 1M trade rows, snappy | 295.19 ms | 2,751.4 ms | 55.66 ms | 5.30 | 49.43 |
| 1M trade rows, zstandard | - | 3,276.3 ms | 51.84 ms | - | 63.20 |
| 1M trade rows, 2 of 6 columns, snappy | 218.95 ms | 2,833.3 ms | 40.41 ms | 5.42 | 70.12 |
| 64K trade rows, deflate | 24.45 ms | 204.8 ms | 5.73 ms | 4.26 | 35.72 |

| write | polars | fastavro | yggdryl | x polars | x fastavro |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1M trade rows, null | 123.48 ms | 2,571.3 ms | 85.28 ms | 1.45 | 30.15 |
| 1M trade rows, deflate | 3,812.26 ms | 4,007.9 ms | 346.61 ms | 11.00 | 11.56 |
| 1M trade rows, snappy | 320.55 ms | 2,597.8 ms | 97.61 ms | 3.28 | 26.61 |
| 64K trade rows, deflate | 115.58 ms | 225.3 ms | 28.54 ms | 4.05 | 7.89 |

What the reader does with the time: a container's blocks are independent once their headers are walked - a length read and jumped per block - so runs of whole blocks decompress and decode on every thread, batches returned in file order; an uncompressed block is decoded where the read's own copy holds it; a varint whose ten bytes are in the buffer is read without a bounds check per byte; and a string column validates its UTF-8 once per batch rather than once per value. A read under a row limit stays on one thread, and a table hands each file its share of `read.parallelism` and `write.parallelism`. The writer cuts rows into blocks of about a megabyte - never more rows than a reader with default limits accepts - resolves each column's Arrow values once per block rather than per cell, and encodes and compresses the blocks on every thread.

```bash
python/.venv/bin/python python/benchmarks/media/avro.py --repeat 5
```

#### Record surface

Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5 150 with rustc 1.96.1 (2026-08-23). The read fixture holds 65,536 rows and four columns; the write fixture holds 4,096 rows, its append and merge base prepared outside the timer.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 26.0 ms | 2.52M rows/s |
| `overwrite_arrow_reader` | 4,096 | 6.10 ms | 671k rows/s |
| `append_arrow_reader` | 4,096 | 12.9 ms | 318k rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 11.4 ms | 358k rows/s |

Opened calls answer from the cache `open` fills; closed calls derive fresh metadata, on the same 65,536-row fixture.

| dimension | fresh | opened |
| --- | ---: | ---: |
| `row_size` | 20.6 us | 83.6 ns |
| `column_size` | 22.3 us | 87.8 ns |
| `read_arrow_field` | 101 us | 72.5 us |

The generic options enum redirects both Avro settings without downcasting or allocation.

| options operation | estimate |
| --- | ---: |
| read the block codec | 9.65 ns |
| set the block codec and a fixed marker | 49.8 ns |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/avro
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/avro
```

#### JavaScript raw codec, historical

One 4,580-byte container of 1,000 three-column rows, fixtures outside the loops, from `npm run bench:codec` on Node 24.18.0, an x86-64 Windows release build (AMD Ryzen 5 150). That script no longer exists and has no replacement, so these numbers stand as history.

| operation | ms/op |
| --- | ---: |
| schema parse / canonical form | 0.136 / 0.003 |
| container decode / resolve / encode | 13.075 / 10.549 / 18.667 |
| first compressed block / decode / resolve | 0.054 / 13.833 / 11.979 |
| single-object decode / encode | 0.018 / 0.087 |

The first-block row includes header parsing and the first lazy `next` but not row decompression; the block decode and resolve rows measure that separately.

#### Against fastavro and PyIceberg, on identical bytes

`scripts/bench_avro_baseline.py` writes one deterministic ten-thousand-entry [Iceberg](#iceberg) manifest (112,246 bytes, statistics included) from Rust, then times three readers over those exact bytes. One containerized x86_64 Linux run: rustc stable release build, CPython 3.11.15, fastavro 1.12.2, pyiceberg 0.11.1.

```text
fastavro 1.12.2:                    67,719 entries/s best (147.7 ms best of 7)
pyiceberg 0.11.1:                   46,790 entries/s best (213.7 ms best of 7)
yggdryl full (release):            101,937 entries/s best ( 98.1 ms best of 7)
yggdryl plan_stats (release):      203,252 entries/s best ( 49.2 ms best of 7)
yggdryl plan_identity (release):   438,596 entries/s best ( 22.8 ms best of 7)
```

| row | call | keeps |
| --- | --- | --- |
| `full` | `read_manifest` | Every field, the way the other two readers do |
| `plan_stats` | `read_manifest_for_plan(handle, true)`, what a filtered scan runs | The value counts, null counts, and bounds that pruning consults; the rest skipped as bytes |
| `plan_identity` | The unfiltered planning read | File identity, partition tuple, and sizes |

On this manifest the planning path is 2.1x the full decode with statistics kept and 4.4x without. The ratios hold at 1,000 and 100,000 entries: `manifest/decode_full`, `manifest/decode_plan_with_stats`, and `manifest/decode_plan_identity_only` in the `media` bench target.

The script needs `fastavro` and `pyiceberg` installed into `python/.venv` and regenerates its fixture itself.

```bash
python/.venv/bin/python scripts/bench_avro_baseline.py
```

#### Codec groups

From the same machine, the five `codec/avro*` groups:

- **Types** (`codec/avro_types`, 10,000 rows each): primitives decode at ~2.9M rows/s and encode at ~3.5M rows/s. Two-string rows decode at ~3.3M rows/s, 18-digit decimals at ~6.7M rows/s, and array of records of maps at ~620K rows/s. The single-object varint floor sits at ~57-65 ns per framed datum.
- **Codec x block size** (`codec/avro_blocks`, 65,536 three-column rows): decode throughput is nearly flat from 1,024 to 65,536 rows per block for every codec. Below ~1,000 rows the per-block header and sync overhead shows. Encoded bytes decode at ~20 MiB/s for snappy, ~12 for deflate, ~9.6 for zstandard, and ~38 for null. Null's bytes are bigger, so compare row rates.
- **Projection** (`codec/avro_projection`, 40 columns, null codec so the skip itself is visible). Reading 3 of 40 columns takes 6.4 ms against 9.4 ms for all 40 over 8,192 rows. The saving is the decode and allocation of the 37 skipped columns, jumped by their length prefixes, never the row read.
- **Resolution** (`codec/avro_resolution`): compiling a five-field plan costs ~533 ns once. Executing it per row beats the direct decode on this shape: 4.12 ms against 4.74 ms for 10,000 rows. The plan skips two writer columns the reader never wanted.

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- codec/avro
```

## Plain text

One record per line, or per framed chain under `framing`; a `rowheader` regex captures typed columns.

`TextLine` exposes the [event identity](../graph.md#contract) and full-width `seqnum`: its UUIDv7 orders by millisecond and row-derived sequence, with the content payload seeded by `crosshashcode`. Its constructor takes a Python integer or JavaScript unsigned 64-bit `bigint` index; assigning Python's writable index recomputes `seqnum` and the identity.

=== "Rust"

    ```rust
    use arrow_array::{Array as _, StringArray, UInt64Array};
    use yggdryl::media::IORecordOptions as _;
    use yggdryl::{IOBase as _, IOMedia as _};
    use yggdryl::holder::Buffer;
    use yggdryl::text::TextOptions;
    use yggdryl::Url;

    let text_source = Buffer::from_bytes(
        b"[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B".to_vec(),
    )
    .with_media_type(Url::from_str("file:///app.log")?.media_type());

    let mut text_options = TextOptions::new();
    text_options.start_rownum = Some(1);
    text_options.set_rowheader(Some(r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "))?;
    text_options.set_framing(true);
    let text_source = text_source.into_text_with(text_options);
    let record_options = text_source.record_options()?;

    let text_batch = text_source
        .read_arrow_reader(&record_options)?
        .next()
        .unwrap()?;
    // The eighteen event columns, then the body, then the header's captures.
    assert_eq!(text_batch.schema().fields().len(), 21);
    // The record's place in its chain is its row number, under `seqnum`.
    assert_eq!(
        text_batch
            .column(14)
            .as_any()
            .downcast_ref::<UInt64Array>()
            .unwrap()
            .values(),
        &[1, 3],
    );
    // The body is the record past the header the reader took off it.
    assert_eq!(
        text_batch
            .column(18)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap()
            .value(0),
        "first\n detail A",
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    with tempfile.TemporaryDirectory() as directory:
        source = pathlib.Path(directory) / "app.log"
        source.write_bytes(
            b"[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B"
        )

        options = TextOptions()
        options.start_rownum = 1
        options.rowheader = r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "
        options.framing = True

        handle = IOBase(source).into_text(options)
        rows = list(handle.read_records())
        assert [row["seqnum"] for row in rows] == [1, 3]
        assert [row["body"] for row in rows] == [
            "first\n detail A",
            "second\n detail B",
        ]
        assert [row["id"] for row in rows] == [7, 9]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, TextOptions } = require('yggdryl')

    const textRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-text-'))
    const textSource = path.join(textRoot, 'app.log')
    fs.writeFileSync(
      textSource,
      '[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B',
    )

    const textOptions = new TextOptions()
    textOptions.startRownum = 1n
    textOptions.rowheader = '^\\[(?<level>[A-Z]+)\\] id=(?<id>\\d+) '
    textOptions.framing = true

    const textHandle = new IOBase(textSource).intoText(textOptions)
    const textRows = [...textHandle.readRecords()]
    assert.deepEqual(textRows.map((row) => row.seqnum), [1n, 3n])
    assert.deepEqual(
      textRows.map((row) => row.body),
      ['first\n detail A', 'second\n detail B'],
    )
    assert.deepEqual(textRows.map((row) => row.id), [7n, 9n])

    fs.rmSync(textRoot, { recursive: true, force: true })
    ```

## JSON

A document is one [`Scalar`](../types/scalar.md); the bindings return native objects unless asked for the scalar.

=== "Rust"

    ```rust
    use yggdryl::json;
    use yggdryl::{from_json_scalar, into_json_scalar, Scalar};

    let value = json::from_utf8(r#"{"symbol":"AAPL","quantity":100}"#)?;
    let encoded = json::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    assert_eq!(encoded, r#"{"quantity":100,"symbol":"AAPL"}"#);

    // The crate-root inferring entry points answer the same value, from text or bytes.
    assert_eq!(from_json_scalar(r#"{"symbol":"AAPL","quantity":100}"#)?, value);
    assert_eq!(into_json_scalar(&value)?, encoded);
    assert_eq!(from_json_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl import json

    natural = json.loads('{"symbol":"AAPL","quantity":100}')
    value = json.loads('{"symbol":"AAPL","quantity":100}', cls=Scalar)
    encoded = json.dumps(value)

    assert value.kind == "struct"
    assert value.as_py() == natural == {"quantity": 100, "symbol": "AAPL"}
    assert encoded == b'{"quantity":100,"symbol":"AAPL"}'
    assert json.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, json } = require('yggdryl')

    const natural = json.loads('{"symbol":"AAPL","quantity":100}')
    const value = json.loads('{"symbol":"AAPL","quantity":100}', { scalar: true })
    const encoded = json.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), '{"quantity":100,"symbol":"AAPL"}')
    assert.deepEqual(json.loads(encoded), natural)
    assert.ok(json.loads(encoded, { scalar: true }).equals(value))
    ```

### JSON performance

One Windows x86_64 release run, one fixture per runtime; compare routes within a runtime, never Python against Node.

| operation | runtime | JSON |
| --- | --- | ---: |
| field class encode | CPython | 150 us |
| field class decode | CPython | 340 us |
| bytes decode | CPython | 19.5 us |
| reader redirect | CPython | 26.8 us |
| writer redirect | CPython | 141 us |
| natural document decode | Node | 9.37 ms |
| natural document emit | Node | 18.0 ms |

```bash
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```

## YAML

=== "Rust"

    ```rust
    use yggdryl::yaml;
    use yggdryl::{from_yaml_scalar, into_yaml_scalar, Scalar};

    let value = yaml::from_utf8("symbol: AAPL\nquantity: 2\n")?;
    let encoded = yaml::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("symbol").and_then(Scalar::as_str),
        Some("AAPL")
    );
    assert_eq!(encoded, "quantity: 2\nsymbol: AAPL\n");

    // The crate-root inferring entry points answer the same value, from text or bytes.
    assert_eq!(from_yaml_scalar("symbol: AAPL\nquantity: 2\n")?, value);
    assert_eq!(into_yaml_scalar(&value)?, encoded);
    assert_eq!(from_yaml_scalar(encoded.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl import yaml

    natural = yaml.loads("symbol: AAPL\nquantity: 2\n")
    value = yaml.loads("symbol: AAPL\nquantity: 2\n", cls=Scalar)
    encoded = yaml.dumps(value)

    assert value.kind == "struct"
    assert value.as_py() == natural == {"quantity": 2, "symbol": "AAPL"}
    assert encoded == b"quantity: 2\nsymbol: AAPL\n"
    assert yaml.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, yaml } = require('yggdryl')

    const natural = yaml.loads('symbol: AAPL\nquantity: 2\n')
    const value = yaml.loads('symbol: AAPL\nquantity: 2\n', { scalar: true })
    const encoded = yaml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(encoded.toString(), 'quantity: 2\nsymbol: AAPL\n')
    assert.deepEqual(yaml.loads(encoded), natural)
    assert.ok(yaml.loads(encoded, { scalar: true }).equals(value))
    ```

### YAML performance

One Windows x86_64 release run, one fixture per runtime; compare routes within a runtime, never Python against Node.

| operation | runtime | YAML |
| --- | --- | ---: |
| field class encode | CPython | 202 us |
| field class decode | CPython | 387 us |
| bytes decode | CPython | 47.6 us |
| reader redirect | CPython | 51.5 us |
| writer redirect | CPython | 200 us |
| natural document decode | Node | 16.5 ms |
| natural document emit | Node | 24.3 ms |

```bash
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```

#### Placeholders

256-entry YAML documents, feature off and on; containerized x86_64 Linux, Criterion medians with 95% intervals.

```text
codec/placeholder/none/off  272.81 us   [271.30 us 274.52 us]
codec/placeholder/none/on   266.07 us   [265.12 us 267.21 us]
codec/placeholder/few/off   265.58 us   [264.58 us 266.86 us]
codec/placeholder/few/on    327.80 us   [325.00 us 330.56 us]
codec/placeholder/most/off  264.84 us   [262.10 us 268.46 us]
codec/placeholder/most/on   386.80 us   [384.48 us 389.17 us]
```

The guard is within run noise; substitution cost about 0.5 us per rebuilt scalar.

```bash
cargo bench -p yggdryl --bench text -- codec/placeholder
```

## TOML

=== "Rust"

    ```rust
    use yggdryl::toml;
    use yggdryl::{from_toml_scalar, into_toml_scalar, Scalar};

    let source = "title = \"yggdryl\"\ncount = 3\n\n[owner]\nname = \"Ada\"\n";
    let value = toml::from_utf8(source)?;
    let encoded = toml::into_utf8(&value)?;

    assert_eq!(
        value.get_key_str("title").and_then(Scalar::as_str),
        Some("yggdryl")
    );
    assert_eq!(toml::from_utf8(&encoded)?, value);

    // The crate-root inferring entry points answer the same Record, from text or bytes.
    assert_eq!(from_toml_scalar(source)?, value);
    assert_eq!(from_toml_scalar(into_toml_scalar(&value)?.as_bytes())?, value);
    ```

=== "Python"

    ```python
    from yggdryl import Scalar
    from yggdryl import toml

    source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    natural = toml.loads(source)
    value = toml.loads(source, cls=Scalar)
    encoded = toml.dumps(value)

    assert value.kind == "struct"
    assert value.as_py() == natural == {
        "count": 3,
        "owner": {"name": "Ada"},
        "title": "yggdryl",
    }
    assert toml.loads(encoded) == natural
    assert toml.loads(encoded, cls=Scalar) == value
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, toml } = require('yggdryl')

    const source = 'title = "yggdryl"\ncount = 3\n\n[owner]\nname = "Ada"\n'
    const natural = toml.loads(source)
    const value = toml.loads(source, { scalar: true })
    const encoded = toml.dumps(value)

    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'struct')
    assert.deepEqual(value.asJs(), natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.deepEqual(toml.loads(encoded), natural)
    assert.ok(toml.loads(encoded, { scalar: true }).equals(value))
    ```

### TOML performance

One Windows x86_64 release run, one fixture per runtime; compare routes within a runtime, never Python against Node.

| operation | runtime | TOML |
| --- | --- | ---: |
| field class encode | CPython | 140 us |
| field class decode | CPython | 363 us |
| bytes decode | CPython | 26.5 us |
| reader redirect | CPython | 27.9 us |
| writer redirect | CPython | 145 us |
| natural document decode | Node | 14.3 ms |
| natural document emit | Node | 15.5 ms |

```bash
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```

## Iceberg

A table lives in one folder: `metadata/` and `data/`, no catalog required.

A scan decodes its files side by side once two of at least 64 KiB qualify (`read.parallel.min-files`, `read.parallel.min-file-size-bytes`), and the files in flight share `read.parallelism` with the columns inside them; a commit shares `write.parallelism` the same way between its partitions and their columns. A partitioned write groups each batch by vectorized keys and computes a partition tuple once per distinct key, not once per row.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{StructType, arrow, DataType};

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use std::sync::Arc;

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-lead");
    let _ = std::fs::remove_dir_all(&path);

    // A table is created in a folder, and a folder is all it ever touches.
    let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // A table that has never been written to has no current snapshot.
    assert!(table.current_snapshot().is_none());
    assert_eq!(table.scan(None)?.count(), 0);

    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2])),
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS")])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let snapshot = table.current_snapshot().expect("a snapshot");
    assert_eq!(snapshot.operation(), "append");
    assert_eq!(table.data_files()?.len(), 2, "one file per venue");

    // Reopening finds the table again, with no catalog in between.
    let reopened = Table::open(LocalFolder::new(&path)?)?;
    let rows: usize = reopened.scan(None)?.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import Table

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")

    # A table is created in a folder, and a folder is all it ever touches.
    table = Table.create(root, schema, ["venue"])

    # A table that has never been written to has no current snapshot.
    assert table.current_snapshot is None
    assert table.scan().read_all().num_rows == 0

    table.append(
        pa.record_batch(
            {"id": [1, 2], "venue": ["XNAS", "XNYS"]},
            schema=pa.schema([
                pa.field("id", pa.int64(), nullable=False),
                pa.field("venue", pa.string()),
            ]),
        )
    )

    assert table.current_snapshot is not None
    assert table.current_snapshot.operation == "append"
    assert len(table.data_files()) == 2, "one file per venue"

    # Reopening finds the table again, with no catalog in between.
    reopened = Table.open(IOBase(root.url.into_path()))
    assert reopened.scan().read_all().num_rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('venue: utf8')], {
      nullable: false,
    })

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')

    // A table is created in a folder, and a folder is all it ever touches.
    const table = iceberg.Table.create(root, schema, ['venue'])

    // A table that has never been written to has no current snapshot.
    assert.equal(table.currentSnapshot, null)
    assert.equal(table.scan().intoTable().numRows, 0)

    table.append(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        venue: arrow.vectorFromArray(['XNAS', 'XNYS'], new arrow.Utf8()),
      }),
    )

    assert.equal(table.currentSnapshot.operation, 'append')
    assert.equal(table.dataFiles().length, 2, 'one file per venue')

    // Reopening finds the table again, with no catalog in between.
    const reopened = iceberg.Table.open(root)
    assert.equal(reopened.scan().intoTable().numRows, 2)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

### Iceberg performance

Release Criterion, Windows 11 Pro 10.0.26200, Ryzen 5 150, rustc 1.96.1. The fastavro and PyIceberg baseline over the same manifest reads sits on [Avro](#avro-performance).

| Metadata operation | Median | Throughput |
| --- | ---: | ---: |
| Parse 100 snapshots and three 50-column schemas | 12.168 ms | 2.8613 MiB/s |
| Expire 99 of 100 snapshots | 9.5145 ms | 3.6592 MiB/s |
| Stable hash of the same metadata | 61.634 us | - |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^metadata/'
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^identity/'
```

The manifest rows share that host and toolchain.

| Manifest operation, 100,000 entries | Median | Throughput |
| --- | ---: | ---: |
| Full official-validated decode | 5.8718 s | 17.031 K entries/s |
| Spec/header only; entries untouched | 190.02 us | 526.26 M nominal entries/s |

```bash
cargo bench --features "parquet iceberg" -p yggdryl --bench media -- '^manifest/'
```

#### Against PyIceberg

One run of `python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5` beside PyIceberg 0.11.1 with its SQLite catalog, on one local warehouse: Intel Xeon @ 2.10 GHz, 4 cores, rustc 1.94.1, CPython 3.11.15, PyArrow 25.0.1, release wheel. Each append writes 1,048,576 six-column rows into a fresh table; both readers then read the table PyIceberg wrote, so they decode the same files, and what both read is compared before anything is timed. The ratio is PyIceberg's median over this crate's, so above one is in this crate's favor.

| operation | unpartitioned | PyIceberg | ratio | 8 partitions | PyIceberg | ratio |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| append | 126.44 ms | 228.71 ms | 1.81 | 249.98 ms | 275.58 ms | 1.10 |
| open | 0.96 ms | 0.72 ms | 0.75 | 1.07 ms | 0.68 ms | 0.64 |
| scan everything | 27.37 ms | 51.83 ms | 1.89 | 31.55 ms | 45.08 ms | 1.43 |
| scan `symbol = 'AAPL'` | 37.08 ms | 63.14 ms | 1.70 | 8.38 ms | 21.25 ms | 2.54 |
| scan `price > 900` | 34.30 ms | 61.63 ms | 1.80 | 41.58 ms | 47.66 ms | 1.15 |
| scan `id, price` | 22.71 ms | 25.58 ms | 1.13 | 19.37 ms | 25.32 ms | 1.31 |

The appends pay one thing PyIceberg's do not: every file published on local storage is flushed to the device before the metadata that names it, so a crash cannot leave the table pointing at a file the disk never received. Opening is the one row behind. PyIceberg is handed the metadata location, while this crate finds it - a table PyIceberg's catalog wrote has no version hint, so the metadata folder is listed - and parses the document twice, once as a value and once through the official crate's validating reader.

```bash
python/.venv/bin/python python/benchmarks/media/iceberg.py --min-time 0.2 --repeat 5
```

#### Iceberg over S3

The same table over the in-process S3 the S3 backend's own suites run on, every request counted: the `s3` group builds a fresh venue-partitioned table per measured commit, scans one of eight partitions, reads the bridge's own `.log` as one object and writes the FIX rows it holds back into a table on the store. Release Criterion `--quick`, sample size 10, on a containerized x86_64 Linux host (Intel Xeon @ 2.10 GHz, 4 cores, 15 GiB; rustc 1.94.1) shared with another build at the time, so the medians are noisier than the request counts, which are exact and pinned in `accounting::iceberg` in `rust/tests/s3/mod_.rs`. The `.log` read is untouched by this work and keeps its six requests; the gap between its two medians is the noise floor of that host, and the FIX row is parsing and enrichment first, remote calls second.

| operation | requests before | requests after | median before | median after |
| --- | ---: | ---: | ---: | ---: |
| append, one partition, 5,000 rows | 25 | 9 | 7.81 ms | 7.41 ms |
| append, eight partitions, 40,000 rows | 67 | 16 | 56.7 ms | 35.0 ms |
| upsert of 10 rows into one partition of eight | 37 | 13 | 16.6 ms | 8.93 ms |
| full scan, eight files | 30 | 10 | 8.70 ms | 5.42 ms |
| pruned scan, one file of eight | 9 | 3 | 4.11 ms | 2.40 ms |
| `.log` object read as text, 2,304 lines | 6 | 6 | 36.2 ms | 25.0 ms |
| FIX rows parsed, enriched and written back | 61 | 20 | 3.99 s | 2.81 s |

Every request left is the metadata chain - the hint, the manifest list, one manifest per commit that survives the summaries, one `GET` per data file - one upload per file a commit writes, and the one listing that claims a version; the loopback timing only shows that nothing else hides between them. On a real store each request is a round trip of 1-20 ms, which is what the counts are worth.

```bash
cargo bench --features "iceberg s3" -p yggdryl --bench media -- 's3/' --quick
```

#### Iceberg on Amazon S3 Tables

`python/benchmarks/media/s3tables.py` is the same question against the real service, beside PyIceberg. It takes a table bucket ARN in `YGGDRYL_S3TABLES_ARN`, has PyIceberg create a table there partitioned by `symbol`, append 65,536 rows in four partitions through the service's catalog - the only door a commit to S3 Tables has - and then opens the same table both ways: PyIceberg through the catalog's REST load, this crate through the warehouse `s3:` location that load answers, with the region the ARN carries and the credentials the catalog vended. Opening the table, a full scan to Arrow, and a scan pruned to one partition of four are each timed on both sides, after the rows both read have been compared; the table is dropped afterwards. The ratio column is PyIceberg's median over this crate's, so above one is in this crate's favor. No table is published here: the run needs an account's own table bucket, and the numbers are those of a network round trip to it, which is why the request counts pinned above are the part that travels.

```bash
YGGDRYL_S3TABLES_ARN=arn:aws:s3tables:<region>:<account>:bucket/<name> python/.venv/bin/python python/benchmarks/media/s3tables.py --min-time 0.2 --repeat 5
```

## Compression

A coding suffix on the name wraps the encoding: the same calls, compressed bytes underneath. `level` is the one setting.

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
    from yggdryl.holder import LocalPath

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
        # LocalPath addresses the stored bytes instead, coding and all.
        stored.append(LocalPath(root / name).read_bytes())

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

### gzip

=== "Rust"

    ```rust
    use yggdryl::gzip;

    let encoded = gzip::dump(b"symbol,price\nAAPL,1\n")?;
    assert_eq!(gzip::load(&encoded)?, b"symbol,price\nAAPL,1\n");
    ```

=== "Python"

    ```python
    import gzip as standard

    from yggdryl import gzip

    encoded = gzip.dumps(b"symbol,price\nAAPL,1\n")
    assert gzip.loads(encoded) == b"symbol,price\nAAPL,1\n"

    # One wire format, so the standard library reads what this wrote and back.
    assert standard.decompress(encoded) == b"symbol,price\nAAPL,1\n"
    assert gzip.loads(standard.compress(b"symbol,price\nAAPL,1\n")) == (
        b"symbol,price\nAAPL,1\n"
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const standard = require('node:zlib')
    const { gzip } = require('yggdryl')

    const payload = Buffer.from('symbol,price\nAAPL,1\n')
    const encoded = gzip.dumps(payload)
    assert.deepEqual(gzip.loads(encoded), payload)

    // One wire format, so node:zlib reads what this wrote and back.
    assert.deepEqual(standard.gunzipSync(encoded), payload)
    assert.deepEqual(gzip.loads(standard.gzipSync(payload)), payload)
    ```

#### gzip performance

One containerized x86_64 Linux run of the Python binding against the standard library's `gzip`, over 1,080,000 bytes of JSON lines.

```text
gzip encode (yggdryl)      0.362 ms   2848.0 MiB/s
gzip encode (stdlib gzip)  3.003 ms    343.0 MiB/s
gzip decode (yggdryl)      0.253 ms   4065.4 MiB/s
gzip decode (stdlib gzip)  0.396 ms   2597.8 MiB/s
```

`zlib-rs` puts the encode 8x ahead; [zlib](#zlib-performance) and [zstd](#zstd-performance) carry their rows from the same run.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```

### zlib

`*_raw` is DEFLATE without the zlib header and trailer.

=== "Rust"

    ```rust
    use yggdryl::zlib;

    let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
    let plain = text.as_bytes();

    let framed = zlib::dump(plain)?;
    let raw = zlib::dump_raw(plain)?;

    assert_eq!(zlib::load(&framed)?, plain);
    assert_eq!(zlib::load_raw(&raw)?, plain);
    assert!(framed.len() < plain.len());

    // The framing is a two-byte header plus a four-byte Adler-32 trailer.
    assert_eq!(framed.len(), raw.len() + 6);

    // Neither decoder accepts the other's bytes.
    assert!(zlib::load(&raw).is_err());
    assert!(zlib::load_raw(&framed).is_err());
    ```

=== "Python"

    ```python
    import zlib as standard

    import pytest

    from yggdryl import zlib

    plain = b"symbol,price\n" + b"AAPL,1\n" * 64

    framed = zlib.dumps(plain)
    raw = zlib.dumps_raw(plain)

    assert zlib.loads(framed) == plain
    assert zlib.loads_raw(raw) == plain
    assert len(framed) < len(plain)

    # The framing is a two-byte header plus a four-byte Adler-32 trailer.
    assert len(framed) == len(raw) + 6

    # Neither decoder accepts the other's bytes.
    with pytest.raises(ValueError):
        zlib.loads(raw)
    with pytest.raises(ValueError):
        zlib.loads_raw(framed)

    # The raw pair is what the standard library spells with a negative window.
    assert standard.decompress(raw, -standard.MAX_WBITS) == plain
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const standard = require('node:zlib')
    const { zlib } = require('yggdryl')

    const plain = Buffer.from('symbol,price\n' + 'AAPL,1\n'.repeat(64))

    const framed = zlib.dumps(plain)
    const raw = zlib.dumpsRaw(plain)

    assert.deepEqual(zlib.loads(framed), plain)
    assert.deepEqual(zlib.loadsRaw(raw), plain)
    assert.ok(framed.length < plain.length)

    // The framing is a two-byte header plus a four-byte Adler-32 trailer.
    assert.equal(framed.length, raw.length + 6)

    // Neither decoder accepts the other's bytes.
    assert.throws(() => zlib.loads(raw))
    assert.throws(() => zlib.loadsRaw(framed))

    // The raw pair is what node:zlib spells inflateRaw.
    assert.deepEqual(standard.inflateRawSync(raw), plain)
    ```

#### zlib performance

`python/benchmarks/coding.py` times `zlib-rs` beside the standard library's zlib over 1,080,000 bytes of JSON lines, one containerized x86_64 Linux run, same wire format. `zlib-rs` puts the encode 9x ahead.

```text
zlib encode (yggdryl)      0.344 ms   2998.3 MiB/s
zlib encode (stdlib zlib)  3.177 ms    324.2 MiB/s
zlib decode (yggdryl)      0.234 ms   4401.1 MiB/s
zlib decode (stdlib zlib)  0.484 ms   2127.9 MiB/s
```

`zlib-rs` level 6 trades a little ratio for speed on repetitive payloads; raise the level when size matters. [gzip](#gzip-performance) and [zstd](#zstd-performance) share this run.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```

### zstd

=== "Rust"

    ```rust
    use yggdryl::zstd;

    let frame = zstd::dump(b"symbol,price\nAAPL,1\n")?;
    assert_eq!(zstd::load(&frame)?, b"symbol,price\nAAPL,1\n");

    // Repetition is what zstd removes.
    let payload = "AAPL,1\n".repeat(64);
    let frame = zstd::dump(payload.as_bytes())?;
    assert!(frame.len() < payload.len());

    // Framing costs bytes, so a short payload comes out larger than it went in.
    assert!(zstd::dump(b"AAPL,1\n")?.len() > 7);

    // A payload that is not a frame is reported, not silently returned.
    assert!(zstd::load(b"definitely not a compressed payload").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import zstd

    frame = zstd.dumps(b"symbol,price\nAAPL,1\n")
    assert zstd.loads(frame) == b"symbol,price\nAAPL,1\n"

    # Repetition is what zstd removes.
    payload = b"AAPL,1\n" * 64
    frame = zstd.dumps(payload)
    assert len(frame) < len(payload)

    # Framing costs bytes, so a short payload comes out larger than it went in.
    assert len(zstd.dumps(b"AAPL,1\n")) > 7

    # A payload that is not a frame is reported, not silently returned.
    with pytest.raises(ValueError):
        zstd.loads(b"definitely not a compressed payload")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { zstd } = require('yggdryl')

    const line = Buffer.from('symbol,price\nAAPL,1\n')
    assert.deepEqual(zstd.loads(zstd.dumps(line)), line)

    // Repetition is what zstd removes.
    const payload = Buffer.from('AAPL,1\n'.repeat(64))
    const frame = zstd.dumps(payload)
    assert.ok(frame.length < payload.length)

    // Framing costs bytes, so a short payload comes out larger than it went in.
    assert.ok(zstd.dumps(Buffer.from('AAPL,1\n')).length > 7)

    // A payload that is not a frame is reported, not silently returned.
    assert.throws(() => zstd.loads(Buffer.from('definitely not a compressed payload')))
    ```

#### zstd performance

One containerized x86_64 Linux run of the Python binding (CPython 3.11) over 1,080,000 bytes of JSON lines.

```text
zstd encode (yggdryl)     14.879 ms     69.2 MiB/s
zstd decode (yggdryl)      0.358 ms   2877.3 MiB/s
```

Standard-library rows need `compression.zstd` (Python 3.14+); on 3.11 the script prints `stdlib compression.zstd unavailable on this interpreter; skipped`.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```

## Charsets

Bytes are decoded once at intake; everything past it is UTF-8.

=== "Rust"

    ```rust
    use yggdryl::Charset;

    // One byte per scalar on the wire, three in UTF-8.
    let wire = b"prix: 12\x80";
    assert_eq!(Charset::Cp1252.decode(wire)?, "prix: 12€");
    assert_eq!(Charset::Cp1252.encode("prix: 12€")?.as_ref(), wire);

    // The same bytes are a different document under a different charset.
    assert_eq!(Charset::Latin1.decode(wire)?, "prix: 12\u{0080}");
    assert_eq!(Charset::from_str("windows-1252")?, Charset::Cp1252);
    ```

=== "Python"

    ```python
    from yggdryl import charset

    wire = b"prix: 12\x80"
    assert charset.decode("windows-1252", wire) == "prix: 12€"
    assert charset.encode("windows-1252", "prix: 12€") == wire

    assert charset.decode("iso-8859-1", wire) == "prix: 12"
    assert charset.canonical_name("cp1252") == "windows-1252"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { charset } = require('yggdryl')

    const wire = Buffer.from('prix: 12\x80', 'latin1')
    assert.equal(charset.decode('windows-1252', wire), 'prix: 12€')
    assert.deepEqual(charset.encode('windows-1252', 'prix: 12€'), wire)

    assert.equal(charset.decode('iso-8859-1', wire), 'prix: 12')
    assert.equal(charset.canonicalName('cp1252'), 'windows-1252')
    ```
