# Media

A handle's name picks the encoding, the compression and the charset; the read and write calls never change.

| Section | Declared by | Build |
| --- | --- | --- |
| [Arrow IPC](#arrow-ipc) | `application/vnd.apache.arrow.stream`, `.arrows` | default |
| [Parquet](#parquet) | `application/vnd.apache.parquet`, `.parquet` | `parquet` feature |
| [Avro](#avro) | `application/avro`, `.avro` | default |
| [Plain text](#plain-text) | `text/plain`, `.txt`, `.log` | default |
| [CSV](#csv) | `text/csv`, `text/tab-separated-values`, `.csv`, `.tsv` | default |
| [JSON](#json) | `application/json`, `application/x-ndjson`, `.json`, `.jsonl` | default |
| [YAML](#yaml) | `application/yaml`, `.yaml` | default |
| [TOML](#toml) | `application/toml`, `.toml` | default |
| [XML](#xml) | `application/xml`, `.xml` | default |
| [XML for Analysis](#xml-for-analysis) | `application/xmla+xml`, `.xmla`; the provider serves catalogs over HTTP | default; the provider's route `http` feature |
| [Excel](#excel) | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet`, `.xlsx` | default |
| [Iceberg](#iceberg) | a table folder | `iceberg` feature |
| [HTTP messages](#http-messages) | `message/http`, `.http` | `http` feature |
| [Compression](#compression) | `.gz`, `.zz`, `.zst` suffix | default |
| [Charsets](#charsets) | `;charset=` parameter | default |

## Read and write

Every record encoding answers the same calls through [`IOMedia`](../holder/index.md#records): `overwrite_records`, `append_records` and `merge_records` write native rows, and the `*_arrow_*` twins read and write Arrow batches. Rows come back as native values through Python `read_records` and JavaScript `readRecords`; Rust has no `read_records` and reads rows through `read_arrow`, a `SerieReader` of one record `Serie` per batch.

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

    from yggdryl import IOBase, scalar

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

    # Instances of a record class are rows of the field their class declares,
    # and each crosses the core's value contract on its way to the column.
    @scalar(frozen=True)
    class Trade:
        id: int
        venue: str

    classes = IOBase(pathlib.Path(tempfile.mkdtemp()) / "classes.arrows")
    classes.overwrite_records([Trade(1, "XNAS"), Trade(2, "XNYS")])
    assert list(classes.read_records(Trade)) == [Trade(1, "XNAS"), Trade(2, "XNYS")]
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

A folder, a location ending in `/` and a glob read as the one table their leaves hold, through the same calls. A folder finds the encoding beneath it; a pattern's suffix names it, so `lake/**/*.parquet` reads as Parquet and only the `.parquet` leaves it matches are read - a marker such as `_SUCCESS` or a note beside the data is not - and `logs/*.log.gz` reads as plain text, each leaf taking off its own coding. Private names - `.venv`, `.config` and anything else starting with a dot - are left out of every walk by default, with the tree beneath them.

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

One `RecordOptions` drives every encoding: the root `field`, `select`, `filter`, `batch_row_size`, `merge_by`, `safe`, `level`, the row bounds `row_offset`, `max_row_size` and `max_byte_size` ([Limits](../holder/index.md#limits)), plus the settings one encoding owns. A JSON, YAML, TOML or XML document is one value rather than a stream of batches, so it has no `RecordOptions`: it reads through `read_arrow` or `read_scalar` and writes, whole, through `write_arrow` or `write_scalar`. The declared `field` and the field a write completes onto are both declarations and cast by [one rule](../types/cast.md#required-columns): a nullable column takes a value it cannot convert as null while `safe` (the default) holds, and a not-null column refuses that value, a null and a missing column by name rather than storing its canonical default.

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
    options.commit_batch_num = 10

    assert str(options.mime_type) == "application/vnd.apache.parquet"
    assert options.name == "row"
    assert [child.name for child in options.field.dtype] == ["id"]
    assert options.select.is_all
    assert options.filter.is_always_true
    assert options.batch_row_size == 1024
    assert options.commit_batch_num == 10

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
    assert.equal(options.withSafe(false).safe, false)
    assert.equal(options.safe, true)
    ```

#### Settings by name

Every record read and write takes `options` and, beside it, the option properties by name - keywords in Python, a plain object in JavaScript (alone in the options position, or after an options value) - each set on a copy of the options by that property's own setter, so a value is checked exactly as an assignment is and the options passed in never change. A value not given (`...` in Python, `undefined` in JavaScript) is skipped; `None`/`null` is a value and clears. A name no setter of that options class owns is not an error: it is skipped with an `UnknownPropertyWarning` naming it and the closest property there is (within a third of its length), so a typo is heard without failing the call - a `warnings` category in Python that a filter can escalate, a process warning (`code: 'YGGDRYL_UNKNOWN_PROPERTY'`) in JavaScript, heard once per process per message. A read-only name (`mime_type`) and another encoding's setting (`rowheader` on CSV) name no settable property; a known name the encoding cannot honour is the setter's own refusal. The same holds for `TextOptions`, `TextLine`, the Iceberg calls and `IcebergOptions`, and an HTTP session's `HttpOptions` properties. Rust sets each property through its typed setter, and `HttpOptions::is_property`, `S3Options::is_property` and `ResolvedFileSystemUri::is_option` answer, from the readers themselves, which names their property doors take.

=== "Rust"

    ```rust
    use yggdryl::http::HttpOptions;
    use yggdryl::media::RecordOptions;
    use yggdryl::MimeType;

    let mut options = RecordOptions::for_mime_type(&MimeType::CSV)?;
    options.set_csv_separator(b';')?;
    assert_eq!(options.csv_separator(), Some(b';'));

    // The by-name door a URL's query and a catalog's properties reach.
    assert!(HttpOptions::is_property("timeout"));
    assert!(!HttpOptions::is_property("timout"));
    ```

=== "Python"

    ```python
    import tempfile
    import warnings
    from pathlib import Path

    from yggdryl import IOBase, RecordOptions, UnknownPropertyWarning

    handle = IOBase(Path(tempfile.mkdtemp()) / "trades.csv")
    handle.write_bytes(b"symbol;price\nAAPL;187\n")

    options = RecordOptions("text/csv")
    table = handle.read_arrow_reader(options=options, separator=";").read_all()
    assert table.column_names == ["symbol", "price"]
    assert options.separator == ","  # set on a copy

    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        handle.read_arrow_field(seperator=";")
    assert issubclass(caught[0].category, UnknownPropertyWarning)
    assert "did you mean 'separator'?" in str(caught[0].message)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, RecordOptions } = require('yggdryl')

    const handle = new IOBase(path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'csv-')), 'trades.csv'))
    handle.writeText('symbol;price\nAAPL;187\n')

    const options = new RecordOptions('text/csv')
    const table = handle.readArrowReader(options, { separator: ';' }).intoTable()
    assert.equal(table.numCols, 2)
    assert.equal(options.separator, ',') // set on a copy

    const heard = []
    const listener = (warning) => heard.push(warning.message)
    process.on('warning', listener)
    handle.readArrowField({ seperator: ';' })
    setImmediate(() => {
      process.off('warning', listener)
      assert.match(heard[0], /did you mean 'separator'\?/)
    })
    ```

## Arrow IPC

The stream carries its schema. A narrower `field` is a column pushdown: skipped columns are never decoded. A stored batch reads back as long as its writer made it unless `batch_row_size` or `batch_byte_size` bounds the read, which cuts it into views over its own buffers.

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

Pages are compressed inside the file (`compression`), and the footer records the codec, so reads name nothing. A coded name such as `.parquet.gz` is refused. Parquet has no union layout, so a union column is refused by name before a byte is written; Arrow IPC holds one, and a [variant](../types/variant.md) column is the semi-structured alternative.

A read's `filter` skips every row group whose footer statistics rule it out before a page is decoded, and the rows of the groups that remain are filtered as always. A column's statistics count only when it is stored as the type the filter reads, and a group holding a null that a declared not-null column refuses is never pruned by that column, so the refusal does not depend on the filter; a floating-point column's minimum and maximum never count, because writers leave NaN out of them, though its null count does. A file of up to a megabyte is read in one request, unless a `max_row_size` without a filter lets its footer be read first; a larger one always has its footer read first. After a footer-first read only the column chunks of the row groups and columns the read keeps are fetched, and chunks less than a megabyte apart come in one request, so a small pruned group or unprojected column between kept ones is read with them. Either way the bytes are copied into memory the reader owns, so rewriting the file while a reader or its batches live is safe. A read yields 65,536-row batches unless `batch_row_size` bounds them; without a filter, never more than `max_row_size` asks for, and under any `max_row_size` - of one file, a folder of them, or an Iceberg table - it decodes lazily, one file at a time on one thread. From a megabyte of column chunks up, it decodes row groups side by side - and, with fewer row groups than threads, each row group's columns - handing batches back in file order, each thread at most sixteen batches ahead of the consumer; split across threads, no batch spans two row groups. A write feeds each row group's column writers as its input arrives - at once on one thread, in feeds of 32 MiB of Arrow input on several, their columns side by side - and the file is byte for byte the one a single thread writes.

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

A container carries its writer schema; a reader schema resolves renames, promotions and defaults. Avro has no dictionary encoding, so a dictionary column is written as the values it encodes.

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

`TextLine` exposes the [event identity](../graph/event.md#identity) and full-width `seqnum`: its UUIDv7 orders by millisecond and row-derived sequence, with the content payload seeded by `crosshashcode`. The row number is the line's [place](../fix/lifecycle.md#a-place-counts-one-instant), where it stands and never what it says: two lines stating the same thing share their `currhashcode` and differ by `curruuid` alone, which sorts the lines of one millisecond by row. Its constructor takes a Python integer or JavaScript unsigned 64-bit `bigint` index; assigning Python's writable index recomputes `seqnum` and the identity.

An object's lines are one chain - they share the object as their cross code - so a read states when that chain began: every line whose own `creaunix` capture states none takes the earliest `currunix` the read has dated a line of its object by so far - never an instant after its own, the first line its own instant. A line the header does not date, of a handle with no time of its own, is dated by nothing and states no creation until a line that is dated; a `creaunix` capture that does not read as an instant stays refused by name. A line built by hand states what it is given.

Each line likewise states, as `prevunix`, the `currunix` the read dated the line cut before it by - none for the first line of an object and none after an undated line, a `prevunix` capture winning - and no `prevuuid`, so no line's `curruuid` or `currhashcode` moves; each object read, each leaf of a folder or glob, starts again, and the [FIX text doors](../fix/arrow.md#a-column-is-the-caller-speaking-per-row) ignore a line's `prevunix`.

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
    // The fifteen event columns, then the body, then the header's captures.
    assert_eq!(text_batch.schema().fields().len(), 18);
    // The record's place is its row number, under `seqnum`.
    assert_eq!(
        text_batch
            .column_by_name("seqnum")
            .expect("the event's place")
            .as_any()
            .downcast_ref::<UInt64Array>()
            .unwrap()
            .values(),
        &[1, 3],
    );
    // The body is the record past the header the reader took off it.
    assert_eq!(
        text_batch
            .column_by_name("body")
            .expect("the record's body")
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

A folder, a location ending in `/` and a glob such as `logs/*.log` read leaf by leaf, through `read_text_lines` and `row_size` as through the record reads: every text leaf beneath them in the listing's order, each the object it is - its own cross code, time, coding and row numbers - and a leaf's last line ends with its leaf. The container's [byte stream](../holder/index.md#streams-and-cursors) runs the same leaves together; a line never reads it.

=== "Rust"

    ```rust
    use yggdryl::Codec;
    use yggdryl::graph::Element as _;
    use yggdryl::holder::Holder;
    use yggdryl::text::{TextOptions, read_text_lines};

    let root = yggdryl::local::LocalFolder::temporary()?.path()?
        .join(format!("yggdryl-docs-text-leaves-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("a.log"), b"a1\na2")?;
    std::fs::write(root.join("b.log.gz"), Codec::Gzip.dump(b"b1\n")?)?;

    let lines = read_text_lines(&Holder::folder(&root)?, &TextOptions::new())?
        .collect::<yggdryl::Result<Vec<_>>>()?;
    // `a2` ends with its leaf, and the gzip leaf is its own decoded object.
    assert_eq!(lines.iter().map(|line| line.body()).collect::<Vec<_>>(), ["a1", "a2", "b1"]);
    assert!(lines[1].get_crosscode().ends_with("a.log"));
    assert!(lines[2].get_crosscode().ends_with("b.log.gz"));

    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl import IOBase, TextOptions

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "a.log").write_bytes(b"a1\na2")
    (root / "b.log.gz").write_bytes(gzip.compress(b"b1\n"))

    lines = list(IOBase(root).read_text_lines(options=TextOptions()))
    # `a2` ends with its leaf, and the gzip leaf is its own decoded object.
    assert [line.body for line in lines] == ["a1", "a2", "b1"]
    assert lines[1].crosscode.endswith("a.log")
    assert lines[2].crosscode.endswith("b.log.gz")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { IOBase, TextOptions } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    fs.writeFileSync(path.join(root, 'a.log'), 'a1\na2')
    fs.writeFileSync(path.join(root, 'b.log.gz'), zlib.gzipSync('b1\n'))

    const lines = [...new IOBase(root).readTextLines(new TextOptions())]
    // `a2` ends with its leaf, and the gzip leaf is its own decoded object.
    assert.deepEqual(lines.map((line) => line.body), ['a1', 'a2', 'b1'])
    assert.ok(lines[1].crosscode.endsWith('a.log'))
    assert.ok(lines[2].crosscode.endsWith('b.log.gz'))

    fs.rmSync(root, { recursive: true, force: true })
    ```

## CSV

RFC 4180 records under the ordinary [record surface](#read-and-write): the header names the columns, a declared `field` is the contract every cell is read under and a bounded sample of the records types them where none is declared, and a `.tsv` name is the same medium under a tab. Compression and the charset are the handle's - `trades.csv.gz` is gzip by name and `;charset=windows-1252` a declared charset - and the dialect is a set of `RecordOptions` properties, so no call takes a format argument.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::local::LocalFolder;
    use yggdryl::media::{IORecordOptions, Media};
    use yggdryl::{Codec, DataType, IOBase, IOMedia, Scalar, StructType};

    struct Trade(i64, Option<&'static str>);

    impl From<Trade> for Scalar {
        fn from(row: Trade) -> Self {
            Scalar::from_sequence([Scalar::from(row.0), row.1.map_or(Scalar::Null, Scalar::from)])
        }
    }

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-docs-csv");
    std::fs::create_dir_all(&root)?;
    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("trade");

    // The name says CSV and gzip: the medium composes over the coding, and the
    // rows cross the declared field's contract on their way to the cells.
    let mut coded = Holder::local(root.join("trades.csv.gz"))?.into_declared_media();
    assert!(matches!(&coded, Holder::Media(media) if matches!(media.as_ref(), Media::Csv(_))));
    let options = coded.record_options()?.with_field(field.clone());
    coded.overwrite_records([Trade(1, Some("AAPL")), Trade(2, None), Trade(3, Some(""))], &options)?;
    let stored = std::fs::read(root.join("trades.csv.gz"))?;
    assert_eq!(&stored[..2], &[0x1F, 0x8B]);
    // A null is the empty cell and the empty text is quoted, so the two read back apart.
    assert_eq!(Codec::Gzip.load(&stored)?, b"id,symbol\n1,AAPL\n2,\n3,\"\"\n");

    // Read back typed, through the same calls on a bare handle: the name picks
    // the coding and the declared field types each cell.
    let mut symbols = Vec::new();
    for records in Holder::local(root.join("trades.csv.gz"))?.read_arrow(Some(&options))? {
        let records = records?;
        let symbol = records.child("symbol").expect("a symbol column");
        for row in 0..symbol.len() {
            symbols.push(symbol.scalar(row)?);
        }
    }
    assert_eq!(symbols, [Scalar::from("AAPL"), Scalar::Null, Scalar::from("")]);

    // A `;` document another writer saved: the separator is a property of one read.
    let semicolon = root.join("trades.csv");
    std::fs::write(&semicolon, b"id;symbol\n1;AAPL\n2;\n")?;
    let handle = Holder::local(&semicolon)?;
    let mut dialect = handle.record_options()?;
    dialect.set_csv_separator(b';')?;
    assert_eq!(
        handle.read_arrow_field(&dialect)?.dtype(),
        &DataType::from_str("struct<id: int64, symbol: utf8>")?
    );
    let batch = handle.read_arrow_reader(&dialect)?.next().expect("one batch")?;
    assert_eq!((batch.num_rows(), batch.num_columns()), (2, 2));
    // Under the default dialect the same header is one column.
    assert_eq!(handle.read_arrow_field(&handle.record_options()?)?.field_len(), 1);

    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    with tempfile.TemporaryDirectory() as directory:
        root = pathlib.Path(directory)
        field = pa.schema([pa.field("id", pa.int64(), nullable=False), pa.field("symbol", pa.string())])

        # The name says CSV and gzip: the handle composes the medium over the
        # coding, and the rows cross the declared field's contract.
        coded = IOBase(root / "trades.csv.gz")
        assert type(coded).__name__ == "Csv"
        options = coded.record_options()
        options.field = field
        rows = [{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": None}, {"id": 3, "symbol": ""}]
        coded.overwrite_records(rows, options=options)
        # A null is the empty cell and the empty text is quoted, so the two read back apart.
        assert gzip.decompress((root / "trades.csv.gz").read_bytes()) == b'id,symbol\n1,AAPL\n2,\n3,""\n'
        assert list(IOBase(root / "trades.csv.gz").read_records(options=options)) == rows

        # A `;` document another writer saved: the separator is a property of one read.
        (root / "trades.csv").write_bytes(b"id;symbol\n1;AAPL\n2;\n")
        handle = IOBase(root / "trades.csv")
        assert [child.name for child in handle.read_arrow_field(separator=";").dtype] == ["id", "symbol"]
        assert handle.read_arrow_reader(separator=";").read_all().num_rows == 2
        assert list(handle.read_records(separator=";")) == [{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": None}]
        # Under the default dialect the same header is one column.
        assert len(handle.read_arrow_field().dtype) == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const zlib = require('node:zlib')
    const { Field, IOBase, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-csv-'))
    const field = fields.struct('trade', [new Field('id', 'int64', false), Field.from('symbol: utf8')], {
      nullable: false,
    })

    // The name says CSV and gzip: the handle composes the medium over the
    // coding, and the rows cross the declared field's contract.
    const coded = new IOBase(path.join(root, 'trades.csv.gz'))
    assert.equal(String(coded.recordOptions().mimeType), 'text/csv')
    const rows = [{ id: 1n, symbol: 'AAPL' }, { id: 2n, symbol: null }, { id: 3n, symbol: '' }]
    coded.overwriteRecords(rows, { field })
    // A null is the empty cell and the empty text is quoted, so the two read back apart.
    assert.equal(
      zlib.gunzipSync(fs.readFileSync(path.join(root, 'trades.csv.gz'))).toString(),
      'id,symbol\n1,AAPL\n2,\n3,""\n',
    )
    assert.deepEqual([...new IOBase(path.join(root, 'trades.csv.gz')).readRecords({ field })], rows)

    // A `;` document another writer saved: the separator is a property of one read.
    fs.writeFileSync(path.join(root, 'trades.csv'), 'id;symbol\n1;AAPL\n2;\n')
    const handle = new IOBase(path.join(root, 'trades.csv'))
    assert.deepEqual(
      Array.from(handle.readArrowField({ separator: ';' }).dtype, (child) => child.name),
      ['id', 'symbol'],
    )
    assert.equal(handle.readArrowReader({ separator: ';' }).intoTable().numRows, 2)
    assert.deepEqual(
      [...handle.readRecords({ separator: ';' })],
      [{ id: 1n, symbol: 'AAPL' }, { id: 2n, symbol: null }],
    )
    // Under the default dialect the same header is one column.
    assert.equal(handle.readArrowField().dtype.length, 1)

    fs.rmSync(root, { recursive: true, force: true })
    ```

The dialect is one `RecordOptions` property per setting. It reads `None`/`null` on another encoding's options, and setting it there is refused.

```text
options.set_csv_separator(b';')?          // Rust RecordOptions: csv_<setting> reads, set_csv_<setting> validates
CsvOptions::new().with_separator(b';')?   // Rust CsvOptions: <setting>, set_<setting>, with_<setting>; tsv() the tab
options.separator = ";"                   // Python property; a byte role is one character or one byte
handle.read_records(separator=";")        // Python: every read and write takes a setting by name, on a copy
options.withSeparator(';')                // JavaScript property or with<Setting>: nullValues, inferRowSize
handle.readRecords({ separator: ';' })    // JavaScript: every read and write takes it in the options object
```

| Setting | Default | Rule |
| --- | --- | --- |
| `separator` | `,`; `\t` under `text/tab-separated-values` | the byte between two cells: one ASCII byte, neither a line break nor a byte another role holds; a tab separator makes the options `text/tab-separated-values` |
| `quote` | `"` | wraps a cell holding the separator, the quote, a line break, a leading or trailing blank, a null spelling (the empty text, by default) or a comment byte opening a record; a quote inside is doubled; `None`/`null` quotes nothing on write, reads a quote as content, and refuses by name a cell that would need quoting |
| `escape` | none | set, the byte after it inside a quoted cell is content, a quote inside is written behind it and the escape byte itself is doubled; unset, a quote inside a quoted cell is doubled (RFC 4180) |
| `comment` | none | a record opening with it is skipped and never counted |
| `header` | on | read: the first record names the columns, an empty name `column_<i>` and a name stated twice refused at `$.header`; off: `column_1`, `column_2`, ...; write: the names are written first, once, an append never repeating them |
| `null_values` | `[""]` | the spellings of an absent value: an unquoted cell spelling one is null and a null is written as the first; a quoted cell is never one, so `""` is the empty text; a spelling listed twice is refused at `$.null_values`, and an empty list leaves a null nothing to be written as, refused at `$[row].<column>` |
| `trim` | off | ASCII blanks around an unquoted cell, and around a quoted one's quotes, are dropped before it is read |
| `infer_row_size` | 1024 | the records sampled to type each column when no field is declared; `0` is refused at `$.infer_row_size` |
| `linesep` | `\n` | Rust only, `CsvOptions::with_linesep`: the terminator a write ends each record with; a read accepts `\n` and `\r\n` whatever it says, and a `\r` alone ends nothing |

### CSV edges

- Inference climbs one ladder per column over the sampled non-null cells - `boolean`, `int64`, `float64`, `date32`, `datetime64(ns, UTC)` (ISO 8601 with a `T` or a space, an offset or `Z` optional, a naive spelling read as UTC), else `utf8` - a cell fitting a rung when that datatype's value door reads it (`true`/`false` in any case, a sign, an exponent, `NaN`, `inf`, blanks around it), and the empty text `""` proving no rung; a column whose sample is all null is `utf8`; every inferred column is nullable and the root is named after the options (`row`). The sampled rows are answered first and the rest streams under that field. An inferred datatype is a reading of the sample, not a contract, so a later cell its column cannot read is refused rather than nulled, under `safe` or not: `$[4].i: expected int64, the datatype the first 2 records (infer_row_size) infer, got "x" in row 6 of <url>; declare the column or raise infer_row_size to sample it`.
- A declared field with a header matches columns by name: a column the field does not name is skipped, a nullable column the header does not state is null, a required one is refused at `$.header`. Without a header the field names the columns by position, and a declared column past the first record's width is missing as an unstated one is: null where nullable, refused at `$.header` where required. Every cell is read through its column's value door, `Field::scalar` over its text, so it reads as the same text does anywhere else. A cell a required column cannot read is refused at `$[row].<column>` naming the cell and the line; a nullable column takes it as null under `safe`, the [cast rule](../types/cast.md#required-columns). A `datetime64` column declared with a zone reads a naive spelling, which its door has no reading for, as a wall clock in that zone, as a text capture's [`autotype`](#plain-text) does.
- A record with more or fewer cells than the header is refused by row - `$[1]: expected 2 cells, got 3 in row 3 of <url>`, the path the 0-based data row and the reason its physical line - the rows before it answered first, nothing widened.
- A blank record is a separator rather than a record, the last record may lack its terminator, a UTF-8 byte-order mark at the start is framing, and bytes after a closing quote are content. A stream ending inside a quoted cell is refused at the record the quote opened in - `$[1]: expected a closing quote, got the end of the stream inside the cell opened in row 3 of <url>`, or `$.header` - by every read and by `row_size`, rather than read as one cell holding every record after it.
- Writing renders every leaf as the text it reads back from: text as itself, numbers, booleans, temporals (`NaN`, `inf`, `-inf` included), codes and UUIDs in their canonical spelling, bytes as base64, a nested value as compact JSON, which a declared nested column reads back; a cell it cannot spell is refused at `$[row].<column>`. A null is its first `null_values` spelling verbatim, so a spelling holding the separator, the quote or a line break, or opening the record with the comment byte, is refused at `$.null_values` where a null is written. A record is never a blank line: the only cell of a record, empty, is written `""` - the empty text, which a column that is not text reads as its null - and a null alone in its record under a text column needs a spelling that is not empty, refused at `$[row].<column>` otherwise.
- A write onto a stored document completes onto its header, never onto its sample, since a CSV stores text: the header's names in its order, each typed as the incoming column of that name, so the rows render as an overwrite renders them; a header column the rows do not carry is written empty, and a column the header does not name is refused at `$.header`, naming it and the header. Without a header the columns are positions: the rows are written as they arrive, and a stored record of another width is refused at `$`, naming both counts. `append` writes after the tail and reads no stored row; `merge` keys through `merge_by` and reads the stored cells under the same columns, refusing at `$[row].<column>` - rather than nulling and writing back lost - a cell its column cannot read.
- While a `Csv` handle is open its schema is cached, and answered only for options reading the document as the ones it was read under: another separator, header, quote, escape, comment, `null_values`, `trim` or `infer_row_size` reads it afresh.
- An empty document declares no schema - `read_arrow_field` is refused at `$.csv` - and reads as no rows; declared, it is the declared schema and no rows.
- `row_size` counts the records in one pass and reads no cell; `column_size` is the header's width; both, like `read_arrow_field`, cost one `pstream_bytes` of the handle ([Call counts](../holder/index.md#call-counts)).

### CSV performance

No table yet: the release run of the command below writes it, on the machine it names, and nothing here is measured in a debug build or edited by hand.

```bash
cargo bench -p yggdryl --bench media -- csv
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

## XML

A document is one [`Scalar`](../types/scalar.md): the record naming its root element. An attribute is an `@name` entry, an element's own text beside attributes or children is `#text`, child elements sharing a name are a sequence in document order, a self-closed element (`<a/>`) is null where one with a body (`<a></a>`) is the empty text, and every leaf is text - XML proves nothing else, so a declared field is what types a document, and it reads one repeated element as one item of a sequence column and an element occurring no time as the empty sequence. Names keep the prefix the document spells; comments, processing instructions and the document type declaration are skipped, and an entity a declaration would have defined is refused by name. Writing is the inverse, one line unless indented, and refuses what XML cannot spell: a root with several entries, a sequence inside a sequence, a key that is not an XML name.

=== "Rust"

    ```rust
    use yggdryl::xml;
    use yggdryl::{from_xml_scalar, from_xml_scalar_with_field, into_xml_scalar, Field, Scalar};

    let source = "<order id=\"7\"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>";
    let value = xml::from_utf8(source)?;

    // The record naming its root: `@` for an attribute, a sequence for a
    // repeated element, null for a self-closed one, text for every leaf.
    let order = value.get_key_str("order").expect("the root element");
    assert_eq!(order.get_key_str("@id").and_then(Scalar::as_str), Some("7"));
    assert_eq!(
        order.get_key_str("leg").and_then(Scalar::as_sequence).map(<[Scalar]>::len),
        Some(2)
    );
    assert_eq!(order.get_key_str("note"), Some(&Scalar::Null));

    // It writes back as the same document, on one line unless indented.
    let encoded = into_xml_scalar(&value)?;
    assert_eq!(
        encoded,
        "<order id=\"7\"><leg>1</leg><leg>2</leg><note/><symbol>AAPL</symbol></order>"
    );
    assert_eq!(from_xml_scalar(encoded.as_bytes())?, value);
    assert_eq!(xml::from_utf8(&xml::into_utf8(&value)?)?, value);

    // A field types the root element's value: one repeated element read once
    // is one item of a sequence column, and text is what the column says.
    let field = Field::from_str(
        "order: struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8> not null",
    )?;
    let typed = from_xml_scalar_with_field(source, &field)?;
    assert_eq!(
        typed,
        Scalar::from_sequence([
            Scalar::from(7),
            Scalar::from("AAPL"),
            Scalar::from_sequence([Scalar::from(1), Scalar::from(2)]),
            Scalar::Null,
        ])
    );
    ```

=== "Python"

    ```python
    from yggdryl import Field, Scalar, xml

    source = '<order id="7"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>'
    natural = xml.loads(source)
    assert natural == {
        "order": {"@id": "7", "symbol": "AAPL", "leg": ["1", "2"], "note": None}
    }

    encoded = xml.dumps(natural)
    assert encoded == (
        b'<order id="7"><leg>1</leg><leg>2</leg><note/><symbol>AAPL</symbol></order>'
    )
    assert xml.loads(encoded) == natural
    assert xml.loads(encoded, cls=Scalar).kind == "struct"

    # A field types the root element's value; a dataclass `cls` is that field.
    field = Field(
        "order",
        "struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8>",
        nullable=False,
    )
    assert xml.loads(source, field=field) == {
        "@id": 7,
        "symbol": "AAPL",
        "leg": [1, 2],
        "note": None,
    }
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, xml } = require('yggdryl')

    const source = '<order id="7"><symbol>AAPL</symbol><leg>1</leg><leg>2</leg><note/></order>'
    const natural = xml.loads(source)
    assert.deepEqual(natural, {
      order: { '@id': '7', symbol: 'AAPL', leg: ['1', '2'], note: null },
    })

    const encoded = xml.dumps(natural)
    assert.ok(Buffer.isBuffer(encoded))
    assert.equal(
      encoded.toString(),
      '<order id="7"><leg>1</leg><leg>2</leg><note/><symbol>AAPL</symbol></order>',
    )
    assert.deepEqual(xml.loads(encoded), natural)
    assert.equal(xml.loads(encoded, { scalar: true }).kind, 'struct')

    // A field types the root element's value.
    const field = new Field(
      'order',
      'struct<@id: int32 not null, symbol: utf8 not null, leg: serie<int32> not null, note: utf8>',
      false,
    )
    assert.deepEqual(xml.loads(source, { field }), {
      '@id': 7,
      symbol: 'AAPL',
      leg: [1, 2],
      note: null,
    })
    ```

As a record medium, a `.xml` handle holds one document element, `data`, with one child element per row named after the root field - `<data><row>...</row></data>` - and reads the rows back as the elements of that name, the root itself when it has none, and no row from an empty root. A charset other than UTF-8 is read off the declaration when neither the handle's media type nor a byte order mark states one, and written back as a declaration for the same reason; the codec itself reads UTF-8, like every other. There is no schema-document door (`DataType::from_xml`, `Field::from_xml`): a schema document types its own leaves, which XML text cannot.

### XML performance

XML is measured where the other formats are: `codec/xml` in the Rust `text` bench, the `XML` rows of `python/benchmarks/text.py`, the `xml/*` rows of `node/benchmarks/text.js`. A table is stated once a release run on the machine the tables above name produces one, and not before.

```bash
cargo bench -p yggdryl --bench text -- codec/xml
python/.venv/bin/python python/benchmarks/text.py --iterations 10000
npm run --prefix node bench:text
```

## XML for Analysis

A `.xmla` handle holds one XML for Analysis 1.1 rowset document - the `xsd:schema` naming its columns and one `<row>` element per row, inside the SOAP 1.1 `DiscoverResponse` a provider would answer or as the bare rowset `root` - and reads and writes it through the same calls as every other medium. The schema travels with the rows: a nullable column is `minOccurs="0"` and absent where its cell is null, a sequence column is `maxOccurs="unbounded"` and one element per item, a struct is the element's children, and a column whose name is not an XML name is written under its `_xHHHH_` escape with `sql:field` keeping the original. Every value is spelled as the XML Schema type its datatype maps to (`xsd:long`, `xsd:double`, `xsd:dateTime`, `uuid`, `xsd:base64Binary`), so a client reads the document without this crate. A declared field types a document written without its schema; without one, the document's own schema is what a read answers, and a `dateTime` column whose values spell a zone lands as `datetime64(us, UTC)`.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let mut handle = Buffer::new().with_media_type(MimeType::XMLA.into());
    let options = handle.record_options()?.with_field(field.clone());
    handle.overwrite_records(
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
        ],
        &options,
    )?;

    // The document carries its schema and one `<row>` per row; a null cell
    // is an absent element.
    let document = String::from_utf8(handle.read_all_bytes()?)?;
    assert!(document.contains("<xsd:element sql:field=\"symbol\" name=\"symbol\" type=\"xsd:string\" minOccurs=\"0\"/>"));
    assert!(document.contains("<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row>"));

    // It reads back through the calls every medium answers.
    let mut rows = 0;
    for batch in handle.read_arrow_reader(&handle.record_options()?)? {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 2);
    assert_eq!(handle.read_arrow_field(&handle.record_options()?)?.fields().len(), 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.xmla")

    # The `.xmla` suffix picks the rowset document; a None makes the column nullable.
    handle.overwrite_records([{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": None}])

    document = handle.read_bytes().decode()
    assert "<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row>" in document
    assert list(handle.read_records()) == [
        {"id": 1, "symbol": "AAPL"},
        {"id": 2, "symbol": None},
    ]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.xmla'))

    // The `.xmla` suffix picks the rowset document; a null makes the column nullable.
    handle.overwriteRecords([{ id: 1n, symbol: 'AAPL' }, { id: 2n, symbol: null }])

    const document = handle.readBytes().toString()
    assert.ok(document.includes('<row><id>1</id><symbol>AAPL</symbol></row><row><id>2</id></row>'))
    assert.deepEqual([...handle.readRecords()], [
      { id: 1n, symbol: 'AAPL' },
      { id: 2n, symbol: null },
    ])

    fs.rmSync(root, { recursive: true, force: true })
    ```

### Provider

`yggdryl::xmla` is also the provider side of the protocol, Rust-only: a [`Service`](https://docs.rs/yggdryl/latest/yggdryl/xmla/struct.Service.html) serves catalogs - a folder of record media is a catalog, each file the folder holds a table, each folder inside it a schema of tables, a folder laid out as an Iceberg table a table wherever it sits (refused by name in a build without the `iceberg` feature), and a ZIP archive a catalog of its members - answering `Discover` with the XMLA schema rowsets (`DISCOVER_DATASOURCES`, `DISCOVER_SCHEMA_ROWSETS`, `DBSCHEMA_CATALOGS`, `DBSCHEMA_SCHEMATA`, `DBSCHEMA_TABLES`, `DBSCHEMA_COLUMNS` and the rest, restrictions applied, and one multidimensional rowset, `MDSCHEMA_CUBES`, each catalog its one cube - what MSOLAP asks for between the catalog list and the tables, and the only `MDSCHEMA_*` rowset a tabular provider answers) and `Execute` by running the statement through the [expression grammar](../expression/index.md) against the table it names, `catalog.schema.table` or the `Catalog` property, refusing a write unless the service was made writable. Every refusal is a SOAP fault carrying the XMLA `<Error>` with a code, a description and the source, answered at HTTP `200` the way the reference providers answer and XMLA clients read one; a header block that demands to be understood and is not a session block earns a `MustUnderstand` fault, and a failure once a streamed answer has begun is reported inside the rowset as `<Messages><Error/></Messages>`. [`Service::route`](https://docs.rs/yggdryl/latest/yggdryl/xmla/struct.Service.html#method.route) puts the service on the crate's [`http::Server`](../holder/index.md#serving-a-handle) at a path: a `GET` answers a short text description of the endpoint and a `HEAD` its head; a `POST` is always answered `200` under `text/xml; charset=utf-8` and `X-Transport-Caps-Negotiation-Flags: 0,0,0,0,0` - a body whose declared content type is not XML earns an immediate `Client` fault under those same headers, and any other body, empty included, is handed to `Service::handle`, its answer written as it is sent so a large `Execute` streams; any other method is the server's own `405` naming `GET, HEAD, POST`. `yggdryl xmla serve` does the same from a terminal, printing the endpoint first.

The binding speaks what the reference clients - MSOLAP, which Excel's Data Connection Wizard and PivotTables use, and ADOMD.NET, which Power Query uses - send: a `Content-Length` or a chunked request body, `Expect: 100-continue`, and MS-SSAS content negotiation. The connection side of that - framing, keep-alive, the interim `100 Continue` answered once the head has passed every check a body is refused on (so a .NET client does not wait its 350 ms), timeouts and bounds, HTTP/2 and HTTP/3, the exchange trace - is the crate's [HTTP server](../holder/index.md#serving-a-handle); what stays XMLA's is the negotiation header itself, stamped on every SOAP answer and never on the `GET` description, plain text XML both ways. A session opens the way those clients open one - an `Execute` carrying `BeginSession` and an empty `<Statement/>`, answered empty under a `Session` block that every later answer carries back, a fault included. `DISCOVER_SCHEMA_ROWSETS` states each rowset's `SchemaGuid` and `RestrictionsMask`, a restriction sent with no value restricts nothing, and `DISCOVER_PROPERTIES` answers the names those clients read before they drive a provider, each with what is true of this one: `ProviderType` 1 (a tabular data provider), `MDXSupport`, `ServerName`, `SQLSupport` 512, `DBMSVersion` `10.50.1600.1` - the SQL Server 2008 R2 RTM build, the release whose XML for Analysis is spoken here and the oldest ADOMD.NET agrees to talk to, `ProviderVersion` staying the crate's own - the `Mdprop*` MDX capability masks - all zero, the statement language being the expression grammar - and the `Dbprop*`/`Ssprop*` properties a client states about itself, echoed back as it set them, a number's own default where it set none, and no cell at all where there is no default - never an empty cell under an `int`. [`ServerOptions::with_trace`](https://docs.rs/yggdryl/latest/yggdryl/http/struct.ServerOptions.html#method.with_trace) - `yggdryl xmla serve --trace <folder>` - writes every exchange as it went over the wire, `NNNN-request.http` as read and `NNNN-response.http` as sent, the interim status and the chunked framing included, numbered from `0000` across every connection: what a client asked is read from the folder, and a request file replays through `Service::handle` with its body.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::IORecordOptions;
    use yggdryl::xmla::{
        Catalog, Discover, Execute, Request, RequestType, Response, Service, ServiceOptions,
    };
    use yggdryl::{DataType, IOBase, IOMedia, Scalar, Serie, StructType};

    let root = std::env::temp_dir().join(format!("yggdryl-xmla-docs-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let field = DataType::from(StructType::from_fields([
        DataType::utf8().required_field("symbol"),
        DataType::Float64.required_field("price"),
    ])?)
    .required_field("row");
    let mut trades = Holder::folder(&root)?.child_by_path("trades.arrows")?;
    let options = trades.record_options()?.with_field(field);
    trades.overwrite_records(
        [
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(187.5_f64)]),
            Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(410.25_f64)]),
        ],
        &options,
    )?;

    // A folder is a catalog; the service answers a request's bytes with a
    // response's bytes, which is what the server puts on the socket.
    let service = Service::new(ServiceOptions::new())
        .with_catalog(Catalog::new("market", Holder::folder(&root)?));
    let discover = Request::from(Discover::new(RequestType::DbschemaTables));
    let answer = service.handle(&discover.into_bytes()?, Vec::new())?;
    let tables = Response::from_bytes(&answer, None)?;
    let names = tables.rows().and_then(|rows| rows.child("TABLE_NAME").cloned());
    assert_eq!(names.map(|column| column.scalar(0)).transpose()?, Some(Scalar::from("trades")));

    // A statement runs through the expression grammar against the table it names.
    let execute = Request::from(Execute::statement(
        "select symbol from market.trades where price > 200",
    ));
    let answer = service.handle(&execute.into_bytes()?, Vec::new())?;
    let selected = Response::from_bytes(&answer, None)?;
    assert_eq!(selected.rows().map(Serie::len), Some(1));
    std::fs::remove_dir_all(&root)?;
    ```

The server is bound the same way in Rust - `let server = http::Server::bind_with("127.0.0.1:8080", ServerOptions::default())?; Arc::new(service).route(&server, "/xmla")?` - and from the command line over any folders a holder resolves, tracing each exchange when asked:

```bash
yggdryl xmla serve market=/data/market reference=s3://bucket/reference --bind 0.0.0.0:8080 --path /xmla
yggdryl xmla serve market=C:\data\market --trace C:\data\trace
```

The `yggdryl` a wheel ships is built with the CLI crate's `iceberg` feature, which `scripts/stage_cli.py` passes, so it reads an Iceberg folder; `cargo build -p yggdryl-cli` alone builds the schema-only core, which lists such a folder as a table and refuses its rows by name.

### Excel as a client

Excel reaches the provider through MSOLAP and through Power Query's ADOMD.NET, and three doors open on a folder catalog `yggdryl xmla serve` puts on a socket. Each was driven from Excel (Microsoft 365, 16.0.20430) against `market=C:\data\market` - Iceberg tables of ten thousand, a hundred thousand and a million trades, a table of every datatype, a `reference` schema - with the exchanges kept under `rust/tests/xmla/fixtures/excel/<door>/` as they went over the wire and replayed through `Service::handle` by `rust/tests/xmla/service.rs`, each answer checked against the captured one by what a client reads: the kind of answer, the columns, the number of rows, the fault code.

- **Power Query, with a query** (`pq-query`, `refresh`). *Données > Nouvelle requête > À partir d'une base de données > SQL Server Analysis Services*: server `http://127.0.0.1:8080/xmla`, database `market`, and the statement in the *Requête MDX ou DAX* box - `select * from market.trades limit 100`. The text crosses unchanged as the `Execute` statement under `Format=Tabular`, the preview types the columns and *Charger* lands the rows; a refresh runs the query again over the pooled connection. The same in M:

    ```text
    AnalysisServices.Database("http://127.0.0.1:8080/xmla", "market",
        [Query = "select * from market.trades limit 100"])
    ```

- **An `.odc` with a command text** (`odc-query`). `Provider=MSOLAP;Data Source=http://127.0.0.1:8080/xmla;Initial Catalog=market;` with `<odc:CommandType>Query</odc:CommandType>` and the statement as `<odc:CommandText>`: opened, Excel lands the rows as an ordinary table in one session of five requests - the leanest path there is, and no Power Query.
- **The Data Connection Wizard** (`wizard`). *Données externes > À partir d'autres sources > À partir d'Analysis Services*: MSOLAP asks `DISCOVER_PROPERTIES`, opens a session, then `DISCOVER_SCHEMA_ROWSETS`, `DBSCHEMA_CATALOGS`, `MDSCHEMA_CUBES` and `DBSCHEMA_TABLES`, lists the catalog as its one cube and saves the `.odc`.

What stays closed, and why. The PivotTable the wizard offers next asks the `MDSCHEMA_*` set and then MDX (`pivottable`); Power Query's navigator - the connector with no query - runs a DMV query, `select [CUBE_NAME], [BASE_CUBE_NAME], [CUBE_CAPTION] from $system.mdschema_cubes where [CUBE_SOURCE] = 1`, and then browses as an MDX or a DAX client (`pq-navigator`); a DAX text, `EVALUATE 'trades'`, is refused by the grammar at byte 0 and Power Query shows the refusal (`pq-dax`). MDX and DAX are not spoken here: the statement language is the [expression grammar](../expression/index.md), and a query is the door.

Two facts the clients read before anything else are stated once, in [`ServiceOptions`](https://docs.rs/yggdryl/latest/yggdryl/xmla/struct.ServiceOptions.html): `DBMSVersion` is `10.50.1600.1` - SQL Server 2008 R2 RTM, the release whose XML for Analysis is spoken here and the oldest ADOMD.NET agrees to talk to - and `ProviderVersion` is the crate's. ADOMD.NET sends every request with `Expect: 100-continue` and chunked, the body opening with a byte-order mark, and a `<Cancel/>` before it reuses a pooled connection, answered empty: no command is ever left running here.

### Behind a reverse proxy

The server speaks cleartext HTTP/1.1 on the socket it binds, and has no TLS on that port. MSOLAP and ADOMD.NET reach an `https` endpoint, and a shared host reaches the provider under a path of its own, through a reverse proxy that terminates TLS and forwards each request over plain HTTP to the socket. What the proxy did to the request is stated to the provider by five flags, each a method of the same name on [`ServerOptions`](../holder/index.md#behind-a-reverse-proxy), which is where the rule for every URL the server states is spelled out:

| flag | `ServerOptions` | states |
| --- | --- | --- |
| `--public-url https://data.example.com/olap` | `with_public_url` | the scheme, host, port and prefix clients use, whatever a request says: the base of every URL the server states and the `URL` `DISCOVER_DATASOURCES` answers |
| `--trusted-proxy 10.0.0.0/8`, repeatable | `with_trusted_proxies` | the peers - IP addresses or CIDR networks - whose forwarded fields are believed; none by default, because any client can write them |
| `--forwarded-header X-Forwarded-Prefix`, repeatable | `with_forwarded_headers` | the forwarded fields read from a trusted peer: `X-Forwarded-For` and `X-Forwarded-Proto` by default, which every proxy below sets on every request. Given, the list replaces the default; name only a field the proxy sets or overwrites, because one it passes through is whatever the client wrote. `Forwarded`, `X-Forwarded-Host`, `-Port` and `-Prefix` are read only when named |
| `--path-prefix /olap` | `with_path_prefix` | the path the proxy leaves in front of `--path`, stripped before routing and carried back on the URLs the server states; a proxy that strips it itself needs none |
| `--read-timeout 75` | `with_read_timeout` | seconds, 1 to 86400, a connection may stay quiet before the server closes it, 30 by default; keep it above the proxy's upstream keep-alive timeout |

```bash
yggdryl xmla serve bronze=/lake/bronze silver=/lake/silver gold=/lake/gold --bind 127.0.0.1:8080 --path /xmla \
  --public-url https://data.example.com/olap --trusted-proxy 10.0.0.0/8 --path-prefix /olap
```

nginx leaves `Forwarded`, `X-Forwarded-Host`, `-Port` and `-Prefix` as the client sent them, which is why none is read unless named: behind this configuration the server needs no `--forwarded-header` at all.

`--public-url` alone is enough when the public endpoint is known: the description a `GET` answers, the `URL` of `DISCOVER_DATASOURCES` and the note printed after the endpoint all state `https://data.example.com/olap/xmla`, and the first line printed stays the socket's own `http://127.0.0.1:8080/xmla`, which is what a script that started the process connects to. Without it, the forwarded fields of a trusted proxy give the description its URL and `DISCOVER_DATASOURCES` states none - the captured Excel exchanges show both clients working without one - so trust the proxy's own address or network and nothing wider, never `0.0.0.0/0`. What holds whatever the proxy is:

- **TLS ends at the proxy.** `https` is the proxy's; it forwards plain HTTP and says so in `X-Forwarded-Proto` (or `Forwarded: proto=https`, once named), which is where the server learns the scheme it is reached under. No certificate is configured on the server.
- **The public host is the proxy's `Host`.** Every proxy below either passes the client's `Host` on or sets it, and that is the host the server states; a host in `X-Forwarded-Host` is read only when `--forwarded-header X-Forwarded-Host` says the proxy sets it.
- **A `POST` is never redirected.** A `GET` or `HEAD` of `/olap/xmla/` - the trailing slash a `location /olap/` block or a copied URL adds - is a `308` to the relative `../xmla`, the query kept; a `POST` there is served by the route. MSOLAP and ADOMD.NET only ever `POST`, and whether either re-sends a body after a `308` is unverified, so a stray slash costs them nothing.
- **Keep the proxy's upstream keep-alive under the server's read timeout.** A proxy reuses an idle connection to the server; the server closes one that has been quiet for `--read-timeout`. When the proxy's idle timeout is the longer of the two it reuses a connection the server has closed and the client sees a `502`, so the proxy's timeout goes below `--read-timeout` (25 s under the default 30), or `--read-timeout` above a timeout the proxy does not let you set (a load balancer's 60 s is `--read-timeout 75`).
- **Speak HTTP/1.1 upstream.** An HTTP/1.0 request - nginx's default upstream version - is answered with a close-delimited body, at one connection per request, and such a body has no way to say it ended short: an `Execute` whose rows fail part way reads as a whole, shorter answer. `proxy_http_version 1.1` with an empty `Connection` header keeps the connections pooled and the answers chunked, where a missing last chunk says so.
- **Stream, and size the body.** A large `Execute` is written as it is sent, so turn response buffering off at the proxy or the first row waits for the last; the largest request body the proxy accepts is `--max-body` (16 MiB by default).

nginx, forwarding the path whole so the server strips the prefix:

```nginx
upstream yggdryl_xmla {
    server 127.0.0.1:8080;
    keepalive 16;
    keepalive_timeout 25s;   # under the server's --read-timeout, 30 s by default
}

server {
    listen 443 ssl;
    server_name data.example.com;
    # ssl_certificate and ssl_certificate_key as usual

    location /olap/ {
        proxy_pass http://yggdryl_xmla;   # no URI part: /olap/xmla crosses whole, hence --path-prefix /olap
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_set_header Host $host;                                   # the public host the server states
        proxy_set_header X-Forwarded-Proto $scheme;                    # overwritten, read by default
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;   # appended, walked from the right
        proxy_request_buffering off;
        proxy_buffering off;
        proxy_read_timeout 300s;
        client_max_body_size 16m;         # = --max-body
    }
}
```

Caddy, which terminates TLS on its own, passes the client's `Host` on, sets `X-Forwarded-For`, `-Proto` and `-Host` itself and strips the prefix, so the server is told the prefix rather than asked to strip it. `header_up` overwrites whatever the client sent in that field, so it is safe to name - `--trusted-proxy 127.0.0.1 --forwarded-header X-Forwarded-For --forwarded-header X-Forwarded-Proto --forwarded-header X-Forwarded-Prefix`:

```caddyfile
data.example.com {
    handle_path /olap/* {
        reverse_proxy 127.0.0.1:8080 {
            header_up X-Forwarded-Prefix /olap
            flush_interval -1
            transport http {
                keepalive 25s
                keepalive_idle_conns 16
            }
        }
    }
}
```

IIS with Application Request Routing and URL Rewrite, the proxy enabled in the ARR module and the forwarded variables allowed in `applicationHost.config` or the site's `web.config`; ARR adds `X-Forwarded-For` on its own, forwards HTTP/1.1 with keep-alive, sends the rewritten `Host`, and buffers answers unless told not to. The rule below sets `X-Forwarded-Proto` and `X-Forwarded-Host` on every request, overwriting the client's, so the host is named too - `--forwarded-header X-Forwarded-For --forwarded-header X-Forwarded-Proto --forwarded-header X-Forwarded-Host`:

```xml
<configuration>
  <system.webServer>
    <rewrite>
      <rules>
        <rule name="yggdryl xmla" stopProcessing="true">
          <match url="^olap/(.*)" />
          <action type="Rewrite" url="http://127.0.0.1:8080/olap/{R:1}" />
          <serverVariables>
            <set name="HTTP_X_FORWARDED_PROTO" value="https" />
            <set name="HTTP_X_FORWARDED_HOST" value="{HTTP_HOST}" />
          </serverVariables>
        </rule>
      </rules>
      <allowedServerVariables>
        <add name="HTTP_X_FORWARDED_PROTO" />
        <add name="HTTP_X_FORWARDED_HOST" />
      </allowedServerVariables>
    </rewrite>
  </system.webServer>
</configuration>
```

```powershell
& "$env:windir\system32\inetsrv\appcmd.exe" set config -section:system.webServer/proxy -responseBufferLimit:0
```

A cloud load balancer - an AWS Application Load Balancer, a Google external HTTP(S) load balancer, an Azure Application Gateway - terminates TLS, passes the client's `Host` on, and sets `X-Forwarded-For` and `X-Forwarded-Proto` on every request it forwards, which are the two read by default; it passes an `X-Forwarded-Host` through as the client wrote it, so name none. Trust the address range the balancer sends from (the subnets it lives in, or the ranges the cloud publishes for it) with `--trusted-proxy`, and raise `--read-timeout` above its idle timeout (60 s on an ALB by default, so `--read-timeout 75`), or state `--public-url` and trust nothing. A balancer that forwards the path whole takes `--path-prefix`; one that rewrites it to `/xmla` takes none.

### Serve Iceberg catalogs as cubes

A [PyIceberg](https://py.iceberg.apache.org/) SQL catalog keeps its table pointers in SQLite and its tables under a warehouse folder, laid out as `<warehouse>/<namespace>/<table>/metadata/*.metadata.json` - which is exactly the catalog, schema and table a folder catalog reads, so a lake of one such catalog per layer is served to Excel with nothing between PyIceberg and the provider but the folders. Three layers - bronze, silver and gold, each a catalog of its own over a warehouse of its own - are three cubes: `MDSCHEMA_CUBES` answers one row per catalog, each namespace is a schema, each table folder a table.

Land the tables through PyIceberg, one SQL catalog per layer with its warehouse a folder beside its database - `sqlite:///lake/silver.db` over `lake/silver`. The block is tagged `ignore` because it needs `pyiceberg`, which the example runner's environment does not carry; it runs as written under an environment that has it:

```{ .python .ignore }
import pathlib
import tempfile

import pyarrow as pa
from pyiceberg.catalog.sql import SqlCatalog

lake = pathlib.Path(tempfile.mkdtemp())


def catalog(layer: str) -> SqlCatalog:
    # The pointers in `<layer>.db`, the tables under `<layer>/<namespace>/<table>/`.
    (lake / layer).mkdir()
    return SqlCatalog(
        layer,
        uri=f"sqlite:///{(lake / f'{layer}.db').as_posix()}",
        warehouse=(lake / layer).as_posix(),
    )


bronze, silver, gold = (catalog(layer) for layer in ("bronze", "silver", "gold"))
for layer in (bronze, silver, gold):
    layer.create_namespace("record_keeping")

log_messages = pa.table(
    {
        "seqnum": pa.array([1, 2, 3], pa.int64()),
        "body": ["8=FIX.4.4|35=D|55=AAPL", "8=FIX.4.4|35=8|55=AAPL", "8=FIX.4.4|35=D|55=MSFT"],
    }
)
bronze.create_table("record_keeping.log_messages", schema=log_messages.schema).append(log_messages)
orders = pa.table(
    {
        "symbol": ["AAPL", "MSFT", "GOOG", "AAPL"],
        "price": pa.array([187.5, 410.25, 141.0, 188.0], pa.float64()),
        "quantity": pa.array([100, 250, 40, 60], pa.int64()),
    }
)
silver.create_table("record_keeping.orders", schema=orders.schema).append(orders)

# What the provider reads: the table folder under the namespace folder, and
# its metadata documents, the highest-numbered one being the current table.
metadata = lake / "silver" / "record_keeping" / "orders" / "metadata"
assert len(list(metadata.glob("0000*.metadata.json"))) == 2
assert gold.list_tables("record_keeping") == []
```

Serve the three warehouses, one catalog each and read-only - never `--writable` over a table PyIceberg manages, because a commit made here would write a new metadata document the SQLite pointer does not name, and the two would fork:

```bash
yggdryl xmla serve bronze=/lake/bronze silver=/lake/silver gold=/lake/gold --bind 0.0.0.0:8080
```

```text
http://127.0.0.1:8080/xmla
· catalog bronze over file:///lake/bronze
· catalog silver over file:///lake/silver
· catalog gold over file:///lake/gold
```

Each layer's warehouse is its own catalog. Serving `/lake` as one catalog would make the layers schemas and read `record_keeping` as a folder table, its Iceberg folders the files of that table. A layer with no table yet - gold above, an empty folder, or a folder that does not exist - is still a cube of no tables, so a dashboard can name it before the first table lands.

What a client then sees is what the [provider](#provider) answers over the same folders, here in one process with no socket between, over tables the crate's own `Table::create` lays out the way PyIceberg does:

```rust
use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table, assign_field_ids};
use yggdryl::local::LocalFolder;
use yggdryl::xmla::{
    Catalog, Discover, Execute, PropertyList, Request, RequestType, Response, Service,
    ServiceOptions,
};
use yggdryl::{DataType, Scalar, Serie, StructType, arrow};

let lake = std::env::temp_dir().join(format!("yggdryl-xmla-lake-{}", std::process::id()));
let _ = std::fs::remove_dir_all(&lake);

// `<layer>/record_keeping/<table>`: a PyIceberg SQL catalog's warehouse layout.
let mut orders = DataType::from(StructType::from_fields([
    DataType::utf8().required_field("symbol"),
    DataType::Float64.required_field("price"),
    DataType::Int64.required_field("quantity"),
])?)
.required_field("row");
assign_field_ids(&mut orders, 1)?;
let folder = LocalFolder::new(lake.join("silver/record_keeping/orders"))?;
let spec = PartitionSpec::identity(0, &orders, &[])?;
let mut table = Table::create(folder, FormatVersion::V2, orders.clone(), spec)?;
let batch = RecordBatch::try_new(
    orders.into_arrow_schema()?,
    vec![
        Arc::new(StringArray::from(vec!["AAPL", "MSFT", "GOOG"])),
        Arc::new(Float64Array::from(vec![187.5, 410.25, 141.0])),
        Arc::new(Int64Array::from(vec![100_i64, 250, 40])),
    ],
)?;
table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;
let mut log_messages = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("seqnum"),
    DataType::utf8().required_field("body"),
])?)
.required_field("row");
assign_field_ids(&mut log_messages, 1)?;
let folder = LocalFolder::new(lake.join("bronze/record_keeping/log_messages"))?;
let spec = PartitionSpec::identity(0, &log_messages, &[])?;
Table::create(folder, FormatVersion::V2, log_messages, spec)?;
std::fs::create_dir_all(lake.join("gold"))?;

// One catalog per layer, as `yggdryl xmla serve bronze=... silver=... gold=...`.
let mut service = Service::new(ServiceOptions::new());
for layer in ["bronze", "silver", "gold"] {
    service = service.with_catalog(Catalog::new(layer, Holder::folder(lake.join(layer))?));
}

// One cube per catalog, in the order they were named.
let cubes = Request::from(Discover::new(RequestType::MdschemaCubes));
let answer = service.handle(&cubes.into_bytes()?, Vec::new())?;
let response = Response::from_bytes(&answer, None)?;
let cubes = response.rows().expect("a rowset");
let names = cubes.child("CUBE_NAME").expect("a column");
let names: Vec<Scalar> = (0..cubes.len()).map(|row| names.scalar(row)).collect::<Result<_, _>>()?;
assert_eq!(names, [Scalar::from("bronze"), Scalar::from("silver"), Scalar::from("gold")]);

// Under a catalog - Excel's database - the tables are the namespace's, and a
// namespace is a schema.
let tables = Request::from(
    Discover::new(RequestType::DbschemaTables)
        .with_properties(PropertyList::new().with("Catalog", "silver")),
);
let answer = service.handle(&tables.into_bytes()?, Vec::new())?;
let response = Response::from_bytes(&answer, None)?;
let tables = response.rows().expect("a rowset");
assert_eq!(tables.len(), 1);
let schema = tables.child("TABLE_SCHEMA").expect("a column").scalar(0)?;
let name = tables.child("TABLE_NAME").expect("a column").scalar(0)?;
assert_eq!((schema, name), (Scalar::from("record_keeping"), Scalar::from("orders")));

// A statement names its table `catalog.schema.table`.
let execute = Request::from(Execute::statement(
    "select symbol, price from silver.record_keeping.orders where price > 150",
));
let answer = service.handle(&execute.into_bytes()?, Vec::new())?;
let response = Response::from_bytes(&answer, None)?;
assert_eq!(response.rows().map(Serie::len), Some(2));
std::fs::remove_dir_all(&lake)?;
```

Excel then opens the [doors above](#excel-as-a-client) on it: Power Query with the server `http://<host>:8080/xmla` - `https://data.example.com/olap/xmla` [behind a proxy](#behind-a-reverse-proxy) - the database `silver` and the statement `select * from silver.record_keeping.orders limit 100`; an `.odc` with `Initial Catalog=silver` and that statement as its command text; the Data Connection Wizard listing `bronze`, `silver` and `gold` as the three cubes. A statement names its table as `catalog.schema.table`, or as `schema.table` under the `Catalog` the connection set - Excel's database - and `record_keeping.orders` with no catalog set is refused by name when several catalogs are served, since each has a `record_keeping`.

What the provider reads, and does not. It opens a table at its highest-numbered `*.metadata.json` and never reads the SQLite pointer, so it agrees with PyIceberg exactly as long as the folder holds one line of history and the pointer names its last document. A table PyIceberg drops stays in the cube until its folder is removed: `drop_table` deletes the row and leaves every file, so the table is still listed and read; `purge_table` deletes the files but leaves the table folder holding an empty `data/` and an empty `metadata/`, which `DBSCHEMA_TABLES` still lists as a table and a `select` of it answers with a fault (`expected a record encoding this build implements ..., got inode/directory`) - so after either call, remove `<warehouse>/<namespace>/<table>/` as well; a table re-created at the same location starts its numbering below the old documents, so the old table is what is served until those are removed; `write.metadata.path` or `write.data.path` pointing outside the table folder is not detected; a nested namespace `a.b` is a folder named `a.b`, a schema name with a dot, and `catalog.a.b.table` is four parts, refused. And writing goes one way: PyIceberg commits land in the folder and are served at the next request, while `--writable` over a PyIceberg-managed table would fork the pointer, so the provider stays read-only over such a lake.

### XML for Analysis performance

Criterion point estimates from a Windows 11 x86_64 release run on an AMD Ryzen 5 150 (6 cores, 23 GiB) with rustc 1.96.1 (2026-09-26). The documents are the ten-thousand-row table of the `media` bench; the provider rows are `Service::handle` over a folder catalog of three IPC tables - the bytes a client sent in, the bytes the server puts on the socket out, with no socket between.

| operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| write a rowset document | 10,000 | 7.56 ms | 1.32M rows/s |
| read a rowset document | 10,000 | 96.9 ms | 103k rows/s |
| write a `Discover` request | - | 1.59 us | - |
| read a `Discover` request | - | 18.5 us | - |
| `DISCOVER_PROPERTIES` | 53 | 233 us | - |
| `DBSCHEMA_CATALOGS` | 1 | 139 us | - |
| `DBSCHEMA_TABLES` | 3 | 1.20 ms | - |
| `DBSCHEMA_COLUMNS` | every column of the 3 | 5.84 ms | - |
| `select * from market.trades_10k` | 10,000 | 10.8 ms | 926k rows/s |
| `select * from market.trades_100k` | 100,000 | 81.4 ms | 1.23M rows/s |
| `select * from market.trades_1m` | 1,000,000 | 822 ms | 1.22M rows/s |

```bash
cargo bench -p yggdryl --bench media -- media/xmla
```

End to end, on the same machine: `yggdryl xmla serve market=C:\data\market` built in release with the `iceberg` feature and no trace, serving the nine-column Iceberg `trades` tables the Excel captures read, and ADOMD.NET 19.84.1 - the client library Power Query drives - executing `select * from market.<table>` over loopback and reading every cell in compiled .NET, warm. Beside it, pyarrow 25.0.1 reading the same tables' Parquet files straight off the disk.

| table | rows | first row, ADOMD.NET | every cell, ADOMD.NET over XML for Analysis | the Parquet files, pyarrow |
| --- | ---: | ---: | ---: | ---: |
| `trades` | 10,000 | 0.02 s | 0.23 s | 0.01 s |
| `trades_100k` | 100,000 | 0.04 s | 2.1 s | 0.01 s |
| `trades_1m` | 1,000,000 | 0.07 s | 19.7 s | 0.06 s |

The provider writes the million rows in 0.82 s; the rest is the client reading them as XML text, which is what the protocol carries. Regenerated by hand: serve the folder as above and time `AdomdCommand.ExecuteReader` draining every row, and `pyarrow.dataset.dataset("<table>/data", format="parquet", partitioning="hive").to_table()` for the baseline.

## Excel

An Office Open XML workbook (`.xlsx`) is a ZIP package of XML parts, and one worksheet of it is the record medium: the first row of the range names the columns, every cell below is a value, and a write renders the part row by row as the batches arrive. The whole workbook is the random-access side of the same medium - [`Workbook`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Workbook.html), [`Sheet`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Sheet.html) and [`Cell`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.Cell.html) - any cell by its `A1` reference, a sheet's rows laid out from a `Serie` and read back as one.

Three facts about the file decide what a read answers. A number cell is a `float64`, because the file stores every number as a double: `1` reads as `1.0`, and a declared `int64` column reads it back as the integer it was written as. A cell's number format is its datatype - a serial under a date format is a `date32`, under a clock a `time32(ms)`, under a date and a clock a `datetime64(ms)`, under `[h]:mm:ss` a `duration64(ms)` - in the workbook's date system, 1900 or 1904. Text is escaped as ECMA-376 spells it: a control character, a carriage return and a literal `_x0041_` are written `_xHHHH_` and read back as themselves, which Excel does and openpyxl leaves unread.

[`ExcelOptions`](https://docs.rs/yggdryl/latest/yggdryl/excel/struct.ExcelOptions.html) adds three settings to the shared ones: `sheet`, the worksheet addressed (the first worksheet by default; a sheet the workbook lacks reads as the empty stream and a write adds it beside the others), `header`, whether the range's first row names the columns (by their letters otherwise), and `range`, the cells addressed (`A3:F`, `B:D`, `2:10`). A write into an opened package keeps every other part as it was, the sheets it does not touch included. A `.xlsx.gz` name is refused: the package is deflated inside, as Parquet is.

=== "Rust"

    ```rust
    use yggdryl::excel::{CellRef, Sheet, Workbook};
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, Serie, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
        DataType::Date32.required_field("traded"),
    ])?)
    .required_field("row");
    let rows = Serie::from_scalars(field.clone(), [
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL"), Scalar::date32(19_723)]),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null, Scalar::date32(19_724)]),
    ])?;

    // The record path: one worksheet, written and read like every medium.
    let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
    let options = handle.record_options()?.with_field(field.clone());
    handle.overwrite_arrow_batch(rows.clone().into_arrow_batch()?, &options)?;
    let read = handle.read_arrow_reader(&options)?.map(|batch| batch.unwrap().num_rows()).sum::<usize>();
    assert_eq!(read, 2);
    // Inferred, the id column is the float64 the file holds.
    let inferred = handle.read_arrow_field(&handle.record_options()?)?;
    assert_eq!(inferred.fields()[0].dtype(), &DataType::Float64);
    assert_eq!(inferred.fields()[2].dtype(), &DataType::Date32);

    // The random-access path: any cell of any sheet, and a sheet as a Serie.
    let mut workbook = Workbook::from_bytes(handle.read_all_bytes()?)?;
    let sheet = workbook.sheet_mut("Sheet1")?;
    assert_eq!(sheet.scalar("B2".parse()?), Scalar::from("AAPL"));
    assert_eq!(sheet.scalar(CellRef::new(2, 1)), Scalar::Null);
    sheet.set_cell("D1".parse()?, "note")?;
    sheet.set_cell("D2".parse()?, 2.5)?;
    let back = sheet.clone().into_serie(Some(&field), true, Default::default())?;
    assert_eq!(back.len(), 2);

    let mut notes = Sheet::new("Notes")?;
    notes.write_serie("A1".parse()?, &rows, true)?;
    workbook.insert_sheet(notes)?;
    let reopened = Workbook::from_bytes(workbook.into_bytes()?)?;
    assert_eq!(reopened.sheet_names(), ["Sheet1", "Notes"]);
    assert_eq!(reopened.sheet("Sheet1")?.scalar("D2".parse()?), Scalar::from(2.5));
    ```

=== "Python"

    ```python
    import datetime
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.excel import Sheet, Workbook

    with tempfile.TemporaryDirectory() as folder:
        path = pathlib.Path(folder) / "trades.xlsx"
        table = pa.table({
            "id": pa.array([1, 2], pa.int64()),
            "symbol": ["AAPL", None],
            "traded": [datetime.date(2024, 1, 1), datetime.date(2024, 1, 2)],
        })

        # The record path: one worksheet, written and read like every medium.
        handle = IOBase(path)
        handle.overwrite_arrow_table(table)
        assert handle.read_arrow_reader(field=table.schema).read_all() == table
        inferred = handle.read_arrow_reader().read_all()
        assert inferred.column("id").to_pylist() == [1.0, 2.0]
        handle.overwrite_arrow_table(pa.table({"note": ["a"]}), sheet="Notes")

        # The random-access path: any cell of any sheet, and a sheet as a Serie.
        workbook = Workbook.open(path)
        assert workbook.sheet_names == ["Sheet1", "Notes"]
        sheet = workbook["Sheet1"]
        assert sheet["B2"].as_py() == "AAPL"
        assert sheet["B3"] is None
        assert sheet["C2"].format == "date"
        sheet["D1"] = "note"
        sheet["D2"] = 2.5
        assert sheet.into_serie(table.schema).into_arrow_table() == table
        workbook.insert_sheet(Sheet.from_serie("Copy", table))
        workbook.write_into(path)
        assert Workbook.open(path)["Sheet1"]["D2"].as_py() == 2.5
        assert IOBase(path).read_arrow_reader(sheet="Copy").read_all().num_rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { DataType, IOBase, Serie, Sheet, Workbook } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-excel-'))
    const file = path.join(root, 'trades.xlsx')
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
    })

    // The record path: one worksheet, written and read like every medium.
    const handle = new IOBase(file)
    handle.overwriteArrowTable(table)
    const field = Serie.fromArrowBatch(table).field
    assert.deepEqual([...handle.readArrowReader(handle.recordOptions().withField(field)).intoTable().getChild('id')], [1n, 2n])
    assert.deepEqual([...handle.readArrowReader().intoTable().getChild('id')], [1, 2])
    handle.overwriteArrowTable(new arrow.Table({ note: arrow.vectorFromArray(['a'], new arrow.Utf8()) }), handle.recordOptions().withSheet('Notes'))

    // The random-access path: any cell of any sheet, and a sheet as a Serie.
    const workbook = Workbook.open(file)
    assert.deepEqual(workbook.sheetNames, ['Sheet1', 'Notes'])
    const sheet = workbook.sheet('Sheet1')
    assert.equal(sheet.cell('B2').value.asJs(), 'AAPL')
    assert.equal(sheet.cell('B3'), null)
    sheet.setCell('D1', 'note')
    sheet.setCell('D2', new DataType('date32').scalar('2024-01-02'))
    assert.equal(sheet.cell('D2').format, 'date')
    assert.equal(sheet.intoSerie(field).asJs().length, 2)
    workbook.insertSheet(Sheet.fromSerie('Copy', table))
    workbook.writeInto(file)
    assert.equal(Workbook.open(file).sheet('Sheet1').cell('D1').value.asJs(), 'note')
    assert.equal(new IOBase(file).readArrowReader(handle.recordOptions().withSheet('Copy')).intoTable().numRows, 2)
    fs.rmSync(root, { recursive: true, force: true })
    ```

What a read costs, in calls to the handle: the package index is the archive's own two reads (its size and the tail holding the directory) and each part read once - the sheet streamed, the shared strings and the styles held for the workbook's life. An inferred read passes the part twice, once to learn the field and once to stream the rows under it; a declared read passes it once. A write of an opened package reads it twice, once for the field the rows are shaped onto and once to carry the other parts across, and renders the sheet as the rows arrive with no row held past its batch. `rust/tests/iobase_calls.rs` pins the record doors' counts and `rust/tests/excel/workbook.rs` the workbook's, and `rust/tests/allocations.rs` pins that a cell read, a reference parse and a range test allocate nothing.

### Excel performance

The benchmark is `cargo bench -p yggdryl --bench media -- excel`, timing the record write and read of ten thousand rows of five columns, the inferred read, one cell through `Workbook`, and a sheet into and from a `Serie`; `python/benchmarks/media/excel.py` times the same file against openpyxl. Neither table is stated here until a release run measures them on a named machine.

## Iceberg

A table lives in one folder: `metadata/` and `data/`, no catalog required.
Iceberg's type strings - `timestamptz`, `fixed[16]`, `list<fixed[16]>`, the
`struct<1: a: optional long>` its reference implementations render - are
[datatype spellings](../types/datatype.md): each primitive reads as the
datatype the table reader maps it to, and a struct member keeps its id and
nullability. A list's or map's string carries no element id or element
nullability, so its child is the grammar's nullable `item`. A v3 `variant` and a v3 `unknown` both read as [`variant`](../types/variant.md); only a table's schema declares a column `unknown`, which it keeps out of its data files and reads back as nulls. A v1 or v2 table refuses both.

A scan decodes its files side by side once two of at least 64 KiB qualify (`read.parallel.min-files`, `read.parallel.min-file-size-bytes`), and the files in flight share `read.parallelism` with the columns inside them; a commit shares `write.parallelism` the same way between its partitions and their columns. A partitioned write groups each batch by vectorized keys and computes a partition tuple once per distinct key, not once per row.

A streamed write with no `commit_batch_num` commits a snapshot each time the batches it holds reach the table's target file size (`write.target-file-size-bytes`, `IcebergOptions`' `target_file_size`) as `yggdryl::arrow::memory_size` measures them, then the remainder, so a stream of any length holds at most one target file of rows before each commit; `commit_batch_num = N` commits every `N` whole batches instead. An overwrite's first commit replaces and the rest append; an append or a merge keeps its intent in every commit.

A partition spec names one transform per field, `Transform` in Rust. The specification's own are read and written as it spells them; three more - `minutes[n]`, `week` and `quarter` - are this crate's own, and no other implementation knows them: Apache Iceberg's Java implementation and PyIceberg read a name they do not know as `unknown` and prune nothing by it, as the specification says of an unknown transform, while iceberg-rust 0.10 refuses a metadata or manifest document naming one, so a table partitioned or sorted by one does not open there. Every time transform is the expression grammar's [epoch function](../expression/functions.md#calendar-parts-and-epoch-periods) of the same name, computed by one rule and floored, so an instant before 1970 lands in its own period - and because a file's tuple names the period every row's source falls in, a filter on the source column prunes files and manifests by it, with no partition column named in the filter, a column two fields read pruning by the tighter of them. A writer that truncated an instant before 1970 toward zero filed it one period late, which Java's reader allows for; a scan here allows for it the same way, so a period at or below zero of the specification's `year`, `month`, `day` and `hour` (of a date, `year` and `month`) also keeps the instants of the period before it. A `bucket` or a `truncate` prunes nothing. A timestamp source of a time transform is counted in microseconds or nanoseconds, the two units Iceberg spells, and one in seconds or milliseconds is refused by all seven alike. Spark's DDL plurals - `years`, `months`, `days`, `hours`, `weeks`, `quarters` - are intake spellings, read and written singular. `minutes[n]` takes its step in brackets as `bucket[n]` does - `minutes[15]` the quarter hour, `minutes[30]` the half hour, `minutes[60]` the hour - with `n` from 1 to 2147483645, and has that one spelling: `minutes[0]`, `minutes(15)`, `minutes[+15]` and a bare `minutes` are refused by name.

| Transform | Also read | Source | Partition value | Grammar function | Whose |
| --- | --- | --- | --- | --- | --- |
| `identity` | | any primitive | the value | | specification |
| `bucket[n]` | `bucket(n)` | int, long, decimal, date, time, timestamp, string, uuid, fixed, binary | `int32` hash bucket | | specification |
| `truncate[w]` | `truncate(w)` | int, long, decimal, string, binary | the value shortened | | specification |
| `year` | `years` | date, timestamp | `int32` years since 1970 | `years(x)` | specification |
| `month` | `months` | date, timestamp | `int32` months since 1970-01 | `months(x)` | specification |
| `day` | `days` | date, timestamp | `date32` the UTC day | `days(x)` | specification |
| `hour` | `hours` | timestamp | `int32` hours since the epoch | `hours(x)` | specification |
| `minutes[n]` | | timestamp | `int32` periods of `n` minutes since the epoch | `minutes(x, n)` | this crate |
| `week` | `weeks` | date, timestamp | `int32` Monday-start weeks since Monday 1969-12-29 | `weeks(x)` | this crate |
| `quarter` | `quarters` | date, timestamp | `int32` quarters since 1970-Q1 | `quarters(x)` | this crate |
| `void` | | any | null | | specification |
| `unknown` | | any | not computed; a spec holding one is not written to | | specification |

In the table's metadata the three cross the Apache Iceberg model this crate validates with as reserved bucket counts above `i32::MAX` - `minutes[n]` as `bucket[2147483648 + n]`, `quarter` as `bucket[4294967294]`, `week` as `bucket[4294967295]` - and come back as themselves; the metadata and manifest files on disk spell the names above, never the bucket, so another writer of this crate reads them. A `bucket[n]` above `i32::MAX`, built in code or stated by a metadata document or a manifest header, is refused by its count rather than read as one of the three, and because a bucket binds to sources a period cannot read, the source of each of the three is judged by the period's own rule - `minutes[n]` over an `int64` or a date is refused naming the transform and the type - wherever a spec or a sort order meets a schema: a table created or read, a spec or an order added. `Transform::from_term` and `Transform::into_term` map a grammar call to its transform and back - `minutes(ts, 15)` is `minutes[15]` - for a partition declaration spelled as an expression; `Transform::function` / `Transform::from_function` are the parameter-free half of that mapping (Rust-only).

A table partitioned by `minutes[15]` writes one data file per quarter hour its rows fall in, and a filter on the timestamp skips the files whose period cannot hold it:

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionField, PartitionSpec, Table, Transform, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, Scalar, StructType, TimeUnit, Timezone, arrow};

    use arrow_array::{Int64Array, RecordBatch, TimestampMicrosecondArray};
    use std::sync::Arc;

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }.required_field("ts"),
    ])?)
    .required_field("row");
    assign_field_ids(&mut schema, 1)?;

    // `minutes[15]` of `ts` (field id 2): one partition per quarter hour since the epoch.
    let spec = PartitionSpec {
        spec_id: 0,
        fields: vec![PartitionField {
            source_id: 2,
            field_id: 1000,
            name: "ts_minutes".into(),
            transform: Transform::from_str("minutes[15]")?,
        }],
    };
    assert!(Transform::from_str("minutes(15)").is_err(), "one spelling");

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-minutes");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;

    // 00:00, 00:14:59, 00:15 and 01:00 of 1970-01-01: three quarter hours.
    let batch = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2, 3, 4])),
            Arc::new(TimestampMicrosecondArray::from(vec![0_i64, 899_000_000, 900_000_000, 3_600_000_000])),
        ],
    )?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let mut periods: Vec<Scalar> = table.data_files()?.into_iter().map(|(file, _)| file.partition[0].clone()).collect();
    periods.sort();
    assert_eq!(periods, vec![Scalar::from(0), Scalar::from(1), Scalar::from(4)]);

    // A filter on `ts` itself skips the files whose period cannot hold it.
    let early = table.plan_matching("ts < '1970-01-01T00:15:00'")?;
    assert_eq!(early.tasks.len(), 1);
    assert_eq!(early.files_skipped(), 2);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import PartitionSpec, Table

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("ts", pa.timestamp("us"), nullable=False),
    ])
    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "ticks")

    # `minutes[15]` of `ts`, the second column (field id 2): one partition per
    # quarter hour since the epoch.
    spec = PartitionSpec.from_json({
        "spec-id": 0,
        "fields": [{"name": "ts_minutes", "transform": "minutes[15]", "source-id": 2, "field-id": 1000}],
    })
    table = Table.create(root, schema, spec)

    # 00:00, 00:14:59, 00:15 and 01:00 of 1970-01-01: three quarter hours.
    table.append(pa.record_batch(
        {"id": [1, 2, 3, 4], "ts": pa.array([0, 899_000_000, 900_000_000, 3_600_000_000], pa.timestamp("us"))},
        schema=schema,
    ))
    assert sorted(file.partition for file, _ in table.data_files()) == [(0,), (1,), (4,)]
    assert table.scan().read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const schema = fields.struct('row', [Field.from('id: int64'), Field.from('ts: timestamp(us)')], {
      nullable: false,
    })
    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'ticks')

    // `minutes[15]` of `ts`, the second column (field id 2): one partition per
    // quarter hour since the epoch.
    const spec = iceberg.PartitionSpec.fromJSON({
      'spec-id': 0,
      fields: [{ name: 'ts_minutes', transform: 'minutes[15]', 'source-id': 2, 'field-id': 1000 }],
    })
    const table = iceberg.Table.create(root, schema, spec)

    // 00:00, 00:14:59, 00:15 and 01:00 of 1970-01-01, as Arrow JS's milliseconds.
    table.append(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n, 3n, 4n], new arrow.Int64()),
        ts: arrow.vectorFromArray([0, 899_000, 900_000, 3_600_000], new arrow.TimestampMicrosecond()),
      }),
    )
    const periods = table
      .dataFiles()
      .map((file) => file.partition[0].asJs())
      .sort((left, right) => left - right)
    assert.deepEqual(periods, [0, 1, 4])
    assert.equal(table.scan().intoTable().numRows, 4)

    fs.rmSync(path.dirname(root), { recursive: true, force: true })
    ```

A table is also created from what its schema declares. `PartitionSpec::from_schema` reads the root's [`PARTITION:by`](../types/protocol.md#partition-columns) - a bare column an identity field, an epoch function over a column its transform, `truncate(col, w)` a truncation, each named by its alias or by the convention (`ts_minutes`, `name_truncate`), anything else refused by name - and `Table::create` reads the root's [`SORT:by`](../types/protocol.md#sort-order) as the default sort order, `SortOrder::for_spec` where it declares none. The declarations are written on the root rather than through `with_partition_by`, because a derived partition value lives in the manifest and not in the rows. `Table::schema()` reports both keys back, `mark_partitions` writing the spec's fields the grammar can spell (a `bucket` has no spelling and is left out) and the default order its keys, so a reopened table says how it partitions and sorts. A partition group whose rows already arrive in the table's order is written as it arrived.

=== "Rust"

    ```rust
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, SortOrder, Table, Transform, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, StructType, TimeUnit, Timezone};

    let mut schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("venue"),
        DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE }.required_field("ts"),
    ])?)
    .required_field("row");
    schema.as_partition_mut().set_by_texts(["venue", "minutes(ts, 15)"])?;
    schema.as_sort_mut().set_by_texts(["ts desc", "id"])?;
    assign_field_ids(&mut schema, 1)?;

    let spec = PartitionSpec::from_schema(1, &schema)?;
    assert_eq!(spec.fields[1].transform, Transform::Minutes(15));
    assert_eq!(spec.fields[1].name, "ts_minutes");
    assert_eq!(SortOrder::from_schema(1, &schema)?.fields.len(), 2);

    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-declared");
    let _ = std::fs::remove_dir_all(&path);
    let table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema, spec)?;
    assert_eq!(table.metadata().default_sort_order()?.fields[0].direction, "desc");
    assert_eq!(table.schema()?.get_metadata("PARTITION:by"), Some(r#"["venue","minutes(ts, 15)"]"#));
    assert_eq!(table.schema()?.get_metadata("SORT:by"), Some(r#"["ts desc","id"]"#));
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import DataType, Field, IOBase
    from yggdryl.iceberg import Table

    schema = Field(
        "row",
        DataType.from_fields([
            Field("id", "int64", nullable=False),
            Field("venue", "utf8", nullable=False),
            Field("ts", "timestamp(us)", nullable=False),
        ]),
        nullable=False,
    )
    schema.partition.by = ["venue", "minutes(ts, 15)"]
    schema.sort.by = ["ts desc", "id"]
    root = pathlib.Path(tempfile.mkdtemp())

    # Omitted, the table partitions and sorts as its schema declares.
    table = Table.create(IOBase(root / "declared"), schema)
    assert [(field.name, field.transform) for field in table.spec.fields] == [
        ("venue", "identity"),
        ("ts_minutes", "minutes[15]"),
    ]
    assert table.schema.partition.by == ["venue", "minutes(ts, 15)"]
    assert table.schema.sort.by == ["ts desc", "id"]

    # Stated, the entries are read by the same rule and replace the declaration.
    stated = Table.create(IOBase(root / "stated"), schema, ["days(ts)", "truncate(venue, 4) as prefix"])
    assert [(field.name, field.transform) for field in stated.spec.fields] == [
        ("ts_day", "day"),
        ("prefix", "truncate[4]"),
    ]

    # `None` partitions nothing, whatever the schema declares.
    assert Table.create(IOBase(root / "flat"), schema, None).spec.is_unpartitioned()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { DataType, Field, iceberg } = require('yggdryl')

    const schema = new Field(
      'row',
      DataType.fromFields([
        new Field('id', 'int64', false),
        new Field('venue', 'utf8', false),
        new Field('ts', 'timestamp(us)', false),
      ]),
      false,
    )
    schema.partition.by = ['venue', 'minutes(ts, 15)']
    schema.sort.by = ['ts desc', 'id']
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))

    // Omitted, the table partitions and sorts as its schema declares.
    const table = iceberg.Table.create(path.join(root, 'declared'), schema)
    assert.deepEqual(
      table.spec.fields.map((field) => [field.name, field.transform]),
      [['venue', 'identity'], ['ts_minutes', 'minutes[15]']],
    )
    assert.deepEqual(table.schema.partition.by, ['venue', 'minutes(ts, 15)'])
    assert.deepEqual(table.schema.sort.by, ['ts desc', 'id'])

    // Stated, the entries are read by the same rule and replace the declaration.
    const stated = iceberg.Table.create(path.join(root, 'stated'), schema, ['days(ts)', 'truncate(venue, 4) as prefix'])
    assert.deepEqual(
      stated.spec.fields.map((field) => [field.name, field.transform]),
      [['ts_day', 'day'], ['prefix', 'truncate[4]']],
    )

    // `null` partitions nothing, whatever the schema declares.
    assert.equal(iceberg.Table.create(path.join(root, 'flat'), schema, null).spec.isUnpartitioned(), true)

    fs.rmSync(root, { recursive: true, force: true })
    ```

`Table.create` and `open_or_create` take the partitioning as a `PartitionSpec` or as `PARTITION:by` entries - Python's `partition_by`, JavaScript's `partitionBy`, each entry its text, in Python a `Term` or a `(term, alias)` pair too - read by `PartitionSpec::from_schema`'s rule, so a refusal names the entry. Omitted, the schema's own `PARTITION:by` is read; `None` in Python and `null` in JavaScript - or an empty list - partition nothing whatever the schema declares. The default sort order is the schema's `SORT:by` either way.

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

### Iceberg schema evolution

A `SchemaUpdate` records column operations - add, rename, drop, promote - and one commit replays them onto the schema that commit attempt reads, so a commit beaten by another writer rebases onto the winner's schema rather than overwriting it. Field IDs are kept and a dropped one is never reused; promotions are `int32 -> int64`, `float32 -> float64`, same-scale decimal widening and, on a v3 table, an `unknown` column to any type, which clears its [`unknown` declaration](../types/variant.md#edges); any other change into or out of `unknown`, `variant` or `binary` is refused. The commit answers the schema id it made current, and an update that recorded nothing writes nothing and answers the current one.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int32Array, RecordBatch};
    use yggdryl::iceberg::{assign_field_ids, FormatVersion, PartitionSpec, SchemaUpdate, Table};
    use yggdryl::local::LocalFolder;
    use yggdryl::{arrow, DataType, StructType};

    let mut schema = DataType::from(StructType::from_fields([DataType::Int32.required_field("id")])?)
        .required_field("row");
    assign_field_ids(&mut schema, 1)?;
    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-iceberg-evolve");
    let _ = std::fs::remove_dir_all(&path);
    let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), PartitionSpec::unpartitioned())?;
    let batch = RecordBatch::try_new(schema.into_arrow_schema()?, vec![Arc::new(Int32Array::from(vec![1]))])?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    let mut update = SchemaUpdate::from_metadata(table.metadata())?;
    update.add_column("", DataType::utf8().nullable_field("note"));
    update.update_type("id", DataType::Int64);
    assert_eq!(table.update_schema(&update)?, 1);

    // Nothing recorded, nothing written: the current id comes back.
    let unchanged = SchemaUpdate::from_metadata(table.metadata())?;
    assert_eq!(table.update_schema(&unchanged)?, 1);

    let first = table.scan(None)?.next().expect("one batch")?;
    assert_eq!(first.column(0).data_type(), &arrow_schema::DataType::Int64);
    assert_eq!(first.column(1).null_count(), 1);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import Field, IOBase
    from yggdryl.iceberg import Table

    root = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades")
    table = Table.create(root, pa.schema([pa.field("id", pa.int32(), nullable=False)]))
    table.append(pa.table({"id": pa.array([1], pa.int32())}))

    schema_id = table.update_schema().add_column("", Field("note", "utf8")).update_type("id", "int64").commit()
    assert schema_id == 1

    # Nothing recorded, nothing written: the current id comes back.
    assert table.update_schema().commit() == 1

    assert [child.name for child in table.schema.dtype] == ["id", "note"]
    assert table.scan().read_all().column("note").to_pylist() == [None]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg } = require('yggdryl')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'trades')
    const table = iceberg.Table.create(root, fields.struct('row', [new Field('id', 'int32', false)], { nullable: false }))
    table.append(new arrow.Table({ id: arrow.vectorFromArray([1], new arrow.Int32()) }))

    const schemaId = table.updateSchema().addColumn('', Field.from('note: utf8')).updateType('id', 'int64').commit()
    assert.equal(schemaId, 1)

    // Nothing recorded, nothing written: the current id comes back.
    assert.equal(table.updateSchema().commit(), 1)

    assert.deepEqual([...table.schema.dtype].map((child) => child.name), ['id', 'note'])
    assert.deepEqual([...table.scan().intoTable().getChild('note')], [null])

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

## HTTP messages

`message/http` (`.http`) is one whole HTTP/1.1 message - a request or a response, its head and its framed body - as RFC 9112 writes it. `Request::from_bytes` and `Response::from_bytes` parse one and `into_bytes` renders it back; a transfer the [HTTP backend](../holder/index.md#http) made is the same value, so a captured exchange and a live one read alike. Behind the `http` feature; the message doors are Rust and JavaScript only.

```text
Response::from_bytes(&[u8]) -> Result<Response>     // status line, headers, framed body; the URL about:blank
Request::from_bytes(&[u8]) -> Result<Request>       // the target joined onto Host
response.into_bytes() / request.into_bytes()        // the message back: names lower case, lexical order
response.into_scalar() -> Result<Scalar>            // {status, reason, version, url, headers, body}
parse_request / parse_response -> (head, body)      // the grammar alone: RequestHead, ResponseHead
render_request / render_response                    // a head and a body back to bytes
decode_chunked(reader) / encode_chunked(writer)     // the chunked framing over std::io
```

=== "Rust"

    ```rust
    use yggdryl::http::{Method, Request, Response, Status};
    use yggdryl::{MimeType, Scalar, Url};

    // The name declares the medium.
    assert_eq!(Url::from_str("file:///capture.http")?.media_type().base(), &MimeType::HTTP);

    let wire = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                 Transfer-Encoding: chunked\r\n\r\n7\r\n{\"a\":1}\r\n0\r\n\r\n";
    let response = Response::from_bytes(wire)?;
    assert_eq!(response.status(), Status::OK);
    assert_eq!(response.text()?, r#"{"a":1}"#);
    // Decoded as RFC 9112 says: the length replaces the transfer coding.
    assert_eq!(response.headers().get("content-length"), Some("7"));
    assert_eq!(response.headers().get("transfer-encoding"), None);
    let record = response.into_scalar()?;
    assert_eq!(record.get_key_str("reason").and_then(Scalar::as_str), Some("OK"));

    // Rendered and parsed again, the message is the same one.
    let again = Response::from_bytes(&response.into_bytes()?)?;
    assert_eq!(again.text()?, response.text()?);

    let request = Request::from_bytes(b"GET /v1/orders?limit=2 HTTP/1.1\r\nHost: api.example.com\r\n\r\n")?;
    assert_eq!(request.method(), Method::Get);
    assert_eq!(request.url().to_string(), "http://api.example.com/v1/orders?limit=2");
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { http } = require('yggdryl')

    const wire = Buffer.from(
      'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n' +
        'Transfer-Encoding: chunked\r\n\r\n7\r\n{"a":1}\r\n0\r\n\r\n',
    )
    const response = http.Response.fromBytes(wire)
    assert.equal(response.statusCode, 200)
    assert.equal(response.text(), '{"a":1}')
    assert.equal(response.headers.get('content-length'), '7')
    assert.equal(http.Response.fromBytes(response.intoBytes()).text(), '{"a":1}')
    ```

A recipient reads what RFC 9112 lets it read, and refuses the rest as `Error::Parse` with target `http message` at the byte position it stopped:

- a line ends in CRLF, a bare LF tolerated as the terminator and a bare CR anywhere else refused; obs-fold is refused;
- a request, status, field or chunk-size line above `MAX_LINE_BYTES` (8192) and a head of more than `MAX_FIELD_LINES` (256) field lines are refused before they are read further;
- `Content-Length` is a decimal every repetition agrees on and never stands beside `Transfer-Encoding`; the one transfer coding read is `chunked`, its extensions ignored and its trailers folded into the headers;
- a `1xx`, `204` or `304` response has no body whatever its headers state, and a response stating no framing reads to the end of the input; the input must be one message;
- a field value's obs-text is read through `Charset::transcribe`, so a legacy byte is its windows-1252 character rather than a refusal;
- a `Content-Encoding` is kept on the body as sent: `bytes`, `text` and `scalar` decode it, `into_bytes` renders it coded, and a coding this crate cannot decode is refused when the message is parsed;
- `parse_response` reads past every interim `1xx` answer other than `101` to the final one, so `Response::from_bytes` answers the status the exchange ended on and `into_bytes` renders only that last answer; `101` is itself final.

A server's trace folder ([Serving a handle](../holder/index.md#serving-a-handle)) is a folder of these documents, one request and one response per exchange.

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
