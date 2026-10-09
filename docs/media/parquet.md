# Parquet

The Apache Parquet file format: columns stored in row groups, pages compressed inside the file, and a footer that records the schema, the codec and each row group's statistics.

## Overview

| | |
| --- | --- |
| Declared by | `application/vnd.apache.parquet`, `.parquet` |
| Build | the `parquet` feature |
| Rust | `yggdryl::parquet`: `Parquet<H>` over any handle, configured by `ParquetOptions`, `PARQUET_CODEC` the [registered medium](index.md#registering-a-medium), the free `read_field`, `read_batch_reader`, `overwrite_arrow_reader` and `read_statistics` over any `IOBase`, and `read_media_statistics` and `read_media_geospatial_statistics` over any media that is one Parquet leaf |
| Python, JavaScript | any `IOBase` whose name declares Parquet, through the [calls every medium answers](index.md#read), plus the footer readers `read_parquet_statistics` / `readParquetStatistics` and `read_parquet_geospatial_statistics` / `readParquetGeospatialStatistics` |
| Settings | `compression`, `max_row_group_size` and `key_value_metadata`, beside the shared [`RecordOptions`](index.md#options); in Rust the fields and setters of `ParquetOptions`, which a handle's options hold as `options.settings::<ParquetOptions>()` ([A medium's own settings](index.md#a-mediums-own-settings)) |
| Refused | a coded name such as `.parquet.gz`, and a union column |
| `uuid` | written as `FIXED_LEN_BYTE_ARRAY(16)` annotated `UUID`, at the root and nested in a struct, a serie or a map; a foreign file's `UUID` column reads as `uuid` |

Pages are compressed inside the file (`compression`), and the footer records the codec, so reads name nothing. A coded name such as `.parquet.gz` is refused. Parquet has no union layout, so a union column is refused by name before a byte is written; [Arrow IPC](ipc.md) holds one, and a [variant](../types/variant.md) column is the semi-structured alternative.

## Read

A read's `filter` skips every row group whose footer statistics rule it out before a page is decoded, and the rows of the groups that remain are filtered as always. A column's statistics count only when it is stored as the type the filter reads, and a group holding a null that a declared not-null column refuses is never pruned by that column, so the refusal does not depend on the filter; a floating-point column's minimum and maximum never count, because writers leave NaN out of them, though its null count does. A narrower `field` skips the column chunks it leaves out.

A read takes the file's last megabyte in one request, which also answers the file's length, so no size is asked first: a file of up to a megabyte is read whole in it, and a larger one has its footer out of it - or out of one more read of the footer's own range, where the footer is longer - then only the column chunks of the row groups and columns the read keeps, chunks less than a megabyte apart coming in one request, so a small pruned group or unprojected column between kept ones is read with them. A `max_row_size` without a filter takes the last 64 KiB instead, so a bounded read of a large file may spare most of it. The schema, the row and column counts and the statistics take the last 64 KiB alone - the footer and the length in one request - and decode no row. That read of the end is `IOBase::read_tail_bytes`: on Amazon S3 and Google Cloud Storage one `GET` with `Range: bytes=-N`, the size taken from its `Content-Range`, while a handle that cannot address a range from its end - an Azure blob, an HTTP resource - asks its size first. An opened `Parquet<H>` hands the footer its `open` read to every read through it, which then fetches only the column chunks. Either way the bytes are copied into memory the reader owns, so rewriting the file while a reader or its batches live is safe.

A read yields 65,536-row batches unless `batch_row_size` bounds them; without a filter, never more than `max_row_size` asks for, and under any `max_row_size` - of one file, a folder of them, or an Iceberg table - it decodes lazily, one file at a time on one thread. From a megabyte of column chunks up, it decodes row groups side by side - and, with fewer row groups than threads, each row group's columns - handing batches back in file order, each thread at most sixteen batches ahead of the consumer; split across threads, no batch spans two row groups.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, IOMedia, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let ids: Vec<i64> = (0..1_000).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    let schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int64Array::from(ids)), Arc::new(StringArray::from(symbols))],
    )?;

    // Four row groups of 250 rows, each with its statistics in the footer.
    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
        .with_options(ParquetOptions::new().with_max_row_group_size(250));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    let statistics = media.read_statistics()?;
    assert_eq!(statistics.num_rows, 1_000);
    assert_eq!(statistics.row_groups.len(), 4);

    // Any media that is one Parquet leaf answers the same footer through the free door.
    assert_eq!(yggdryl::parquet::read_media_statistics(&media)?.num_rows, 1_000);

    // The filter rules three row groups out from the footer alone; the last
    // one is decoded and its rows filtered.
    let late = options.clone().with_filter("id >= 900")?;
    let rows: usize = media.read_arrow_reader(&late)?.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(rows, 100);

    // A narrower field skips the column chunks it leaves out.
    let wanted = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let projected = media.read_arrow_reader(&options.clone().with_field(wanted))?;
    assert_eq!(projected.schema().fields().len(), 1);
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
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")

    # Four row groups of 250 rows, each with its statistics in the footer.
    handle.overwrite_arrow_table(
        pa.table({"id": list(range(1_000)), "symbol": ["AAPL"] * 1_000}, schema=schema),
        max_row_group_size=250,
    )
    statistics = handle.read_parquet_statistics()
    assert statistics["num_rows"] == 1_000
    assert [group["num_rows"] for group in statistics["row_groups"]] == [250] * 4

    # The filter rules three row groups out from the footer alone; the last
    # one is decoded and its rows filtered.
    assert handle.read_arrow_reader(filter="id >= 900").read_all().num_rows == 100

    # A narrower field skips the column chunks it leaves out.
    wanted = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    assert handle.read_arrow_reader(field=wanted).schema.names == ["id"]

    # A row bound cuts what a read yields.
    assert [batch.num_rows for batch in handle.read_arrow_reader(batch_row_size=400)] == [
        400,
        400,
        200,
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, IOBase, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    const ids = Array.from({ length: 1_000 }, (_, index) => BigInt(index))

    // Four row groups of 250 rows, each with its statistics in the footer.
    handle.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray(ids, new arrow.Int64()),
        symbol: arrow.vectorFromArray(ids.map(() => 'AAPL'), new arrow.Utf8()),
      }),
      { maxRowGroupSize: 250 },
    )
    const statistics = handle.readParquetStatistics()
    assert.equal(statistics.num_rows, 1_000)
    assert.equal(statistics.row_groups.length, 4)

    // The filter rules three row groups out from the footer alone; the last
    // one is decoded and its rows filtered.
    assert.equal(handle.readArrowReader({ filter: 'id >= 900' }).intoTable().numRows, 100)

    // A narrower field skips the column chunks it leaves out.
    const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    assert.equal(handle.readArrowReader({ field: wanted }).intoTable().numCols, 1)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Write

`compression` names the page codec as the Parquet format does - `uncompressed`, `snappy`, `gzip(n)`, `brotli(n)`, `lz4`, `lz4_raw` or `zstd(n)` - and the footer records it, so the read side names nothing. `max_row_group_size` bounds a row group (1,048,576 rows by default) and `key_value_metadata` adds its pairs to the footer. A write feeds each row group's column writers as its input arrives - at once on one thread, in feeds of 32 MiB of Arrow input on several, their columns side by side - and the file is byte for byte the one a single thread writes. A coding around the whole file is refused before anything is published, because no Parquet reader could open it.

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
        sizes.append(handle.size())

    assert sizes[0] > sizes[1] and sizes[0] > sizes[2], sizes

    # A coding around the whole file is refused, and nothing is published.
    coded = IOBase(root / "trades.parquet.gz")
    with pytest.raises(ValueError, match="parquet compresses"):
        coded.overwrite_arrow_table(table)
    assert coded.size() == 0
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
      sizes.push(handle.size())
    }

    assert.ok(sizes[0] > sizes[1] && sizes[0] > sizes[2], sizes.join())

    // A coding around the whole file is refused, and nothing is published.
    const coded = new IOBase(path.join(root, 'trades.parquet.gz'))
    assert.throws(() => coded.overwriteArrowTable(table), /parquet compresses/)
    assert.equal(coded.size(), 0)

    fs.rmSync(root, { recursive: true, force: true })
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

One containerized x86_64 Linux run of `python/benchmarks/media.py` with `--min-time 0.1 --repeat 3`: 65,536 rows, 4 columns, 8 batches; Intel Xeon @ 2.10 GHz, 4 cores, rustc 1.94.1, CPython 3.11.15, PyArrow 25.0.1, release wheel.

```text
parquet write reader             6.662 ms    9.8M rows/s
PyArrow parquet write baseline   8.768 ms    7.5M rows/s
parquet read whole               2.361 ms   27.8M rows/s
PyArrow parquet read baseline    2.500 ms   26.2M rows/s
```

On a file this small the read and the write are both ahead of PyArrow; [Streaming against PyArrow](#streaming-against-pyarrow) measures the sizes where the threads pay. [Arrow IPC](ipc.md#performance) carries that encoding's rows from the same run.

```bash
python/.venv/bin/python python/benchmarks/media.py --filter "parquet write" --filter "parquet read whole" --filter "parquet read subset" --filter "parquet read records" --filter "parquet row size" --filter "parquet column size" --filter "PyArrow parquet"
```

### Streaming against PyArrow

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

### Footer statistics

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
