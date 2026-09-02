# Apache Parquet

Read and write Apache Parquet files, and their footer statistics, over any handle.

At handle level, overwrite, append, and keyed merge use the shared
[canonical record-write signatures](io.md#canonical-record-write-signatures).
The free `parquet::overwrite_arrow_reader` function below remains the one
complete-file encoder those intents publish through; the stateful wrapper uses
`overwrite_arrow_reader` like every other media handle.

!!! note "All three"
    Python and JavaScript reach the encoding through [`IOBase`](io.md)'s record
    methods, which cover reading, writing, column pushdown, row groups, and
    footer statistics. The stateful `Parquet` wrapper and encoding free
    functions stay Rust-only; the inferred handle surface is shared by all
    three languages.

## Arrow batch reads and writes

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOBase, IOMedia};
    use yggdryl::{DataType, Url};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = |ids: Vec<i64>, symbols: Vec<Option<&str>>| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
            ],
        )
    };

    // The name decides Parquet; the methods name the write intent.
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.parquet")?.media_type());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![1, 2], vec![Some("AAPL"), Some("MSFT")])?],
        ),
        &options,
    )?;
    handle.append_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![3], vec![Some("GOOG")])?],
        ),
        &options,
    )?;
    handle.merge_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![2, 4], vec![Some("NVDA"), None])?],
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
    import pyarrow.parquet as pq

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    batch = lambda ids, symbols: pa.record_batch(
        {"id": ids, "symbol": symbols}, schema=schema
    )

    path = pathlib.Path(tempfile.mkdtemp()) / "trades.parquet"
    with IOBase(path) as handle:
        handle.overwrite_arrow_batch(batch([1, 2], ["AAPL", "MSFT"]))
        handle.append_arrow_batch(batch([3], ["GOOG"]))

        merging = handle.record_options()
        merging.merge_by_names = ["id"]
        handle.merge_arrow_batch(
            batch([2, 4], ["NVDA", None]), options=merging
        )

        assert handle.read_arrow_reader().read_all().num_rows == 4

    assert pq.read_table(path).num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const rows = (ids, symbols) => new arrow.Table({
      id: arrow.vectorFromArray(ids.map(BigInt), new arrow.Int64()),
      symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
    })

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteArrowTable(rows([1, 2], ['AAPL', 'MSFT']))
    handle.appendArrowTable(rows([3], ['GOOG']))
    handle.mergeArrowTable(
      rows([2, 4], ['NVDA', null]),
      handle.recordOptions().withMergeByNames(['id']),
    )

    assert.equal(handle.readArrowReader().intoTable().numRows, 4)
    assert.deepEqual([...handle.readBytes().subarray(0, 4)], [...Buffer.from('PAR1')])

    fs.rmSync(root, { recursive: true, force: true })
    ```

Parquet answers the same explicit overwrite, append, and keyed merge intents
as every other encoding. Their [canonical signatures and streaming
rules](io.md#canonical-record-write-signatures) apply without a format
argument: the handle's media type selects Parquet, while `merge_by_names`
supplies row-identity keys only.

### Measured batch operations

The read fixture contains 65,536 rows and four columns. The write fixture contains 4,096 rows;
Criterion prepares the stored side for append and keyed merge outside the timer. Keyed merge is
the upsert operation: matching `id` rows are updated and misses are inserted.

| batch operation | rows | estimate | throughput |
| --- | ---: | ---: | ---: |
| read and drain `read_arrow_reader` | 65,536 | 18.6 ms | 3.52M rows/s |
| `overwrite_arrow_reader` | 4,096 | 3.91 ms | 1.05M rows/s |
| `append_arrow_reader` | 4,096 | 8.06 ms | 508k rows/s |
| keyed `merge_arrow_reader` (upsert) | 4,096 | 9.03 ms | 453k rows/s |

These are Criterion point estimates from a Windows x86_64 release smoke run on an AMD Ryzen 5
150 with rustc 1.96.1 (2026-08-23). Regenerate them on the deployment host with
`io_dimensions/parquet/read_rows` and `io_write_stateful/parquet`; the longer PyArrow comparison
remains in [Against PyArrow](#against-pyarrow).

### Dimensions and opened sessions

`row_size` and `column_size` range-read only the eight-byte tail and footer; no row group or column
page is decoded. They describe the whole file, ignoring selection, filters, and limits. Closed calls
read a fresh footer; `open` retains the inferred Parquet wrapper and footer until `close`, and writes
invalidate it.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::generic::Holder;
    use yggdryl::io::{Buffer, IOBase, IOMedia};
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

The same 65,536-row fixture measured fresh/opened `row_size` at 8.95 us/9.12 ns,
fresh/opened `column_size` at 18.1 us/6.86 ns, and fresh/opened `read_arrow_field` at
297 us/119 us. Regenerate with
`cargo bench -p yggdryl --bench io --all-features -- io_dimensions/parquet`.

Everything on this page is behind the non-default `parquet` feature. The codec is version-locked to
the pinned Arrow release and pulls in a thrift and compression stack a schema-only consumer never
touches, so it is opted into rather than carried. Without it the module does not exist, and
[`RecordOptions::for_mime_type`](generic.md) reports `application/vnd.apache.parquet` as an encoding
this build does not implement instead of guessing one.

`Parquet<H>` binds one file to its handle, default settings, and opened footer
cache. `record_options` returns those defaults for the canonical `IOMedia`
calls: `overwrite_arrow_reader` consumes an [`arrow::BatchReader`](arrow.md),
while `read_arrow_reader` returns one and `read_arrow_field` returns its
canonical non-null struct root [`Field`](field.md). `arrow::batch_reader` turns
batches already in hand into the same streaming shape; every pulled batch is
checked against the reader's field and errors name its index.

The specialized `read_arrow_schema` returns the Arrow schema and
`read_statistics` returns the footer. The encoding seams also exist as free functions - `parquet::read_arrow_schema`,
`read_field`, `read_batch_reader`, `overwrite_arrow_reader`, `read_statistics` - taking the handle
explicitly, and a `&ParquetOptions` where the settings matter.

Both bindings call the same operations on the handle itself - Python's
`read_arrow_reader` / `overwrite_arrow_reader` and JavaScript's
`readArrowReader` / `overwriteArrowReader` - so a file this large never becomes a table on either side unless a caller asks
for one. Python exchanges `pyarrow.RecordBatchReader` values across the Arrow C Stream interface;
JavaScript exchanges Apache Arrow JS values over the copied Arrow IPC boundary, one batch per stream.
`record_options()`/`recordOptions()` is the Parquet settings value, carrying `compression`,
`max_row_group_size`, and `key_value_metadata` next to the shared settings.

## Column pushdown

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Float64Array, Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOMedia};
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let stored = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Utf8.required_field("venue"),
    ])?
    .required_field("row");
    let arrow_schema = stored.into_arrow_schema()?;

    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            Arc::new(Float64Array::from(vec![1.5, 2.5])),
            Arc::new(StringArray::from(vec!["XNAS", "XNAS"])),
        ],
    )?;

    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;

    // Two of the four columns, named by a root Field of its own.
    let wanted = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])?
    .required_field("row");

    let projected = media.read_arrow_reader(&options.clone().with_field(wanted))?;
    assert_eq!(projected.schema().fields().len(), 2);
    let read = projected.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(read[0].num_columns(), 2);

    // The file is unchanged: it still stores all four.
    assert_eq!(media.read_arrow_schema()?.fields().len(), 4);
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
        pa.field("price", pa.float64(), nullable=False),
        pa.field("venue", pa.string(), nullable=False),
    ])
    rows = 4_096
    batch = pa.record_batch(
        {
            "id": list(range(rows)),
            "symbol": ["AAPL"] * rows,
            "price": [1.5] * rows,
            "venue": ["XNAS"] * rows,
        },
        schema=stored,
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
    handle.overwrite_arrow_batch(batch)

    # Two of the four columns, declared through the centralized options field.
    options = handle.record_options()
    options.field = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("price", pa.float64(), nullable=False),
    ])
    projected = handle.read_arrow_reader(options=options).read_all()
    assert projected.column_names == ["id", "price"]

    # Less is read, and the bytes say so rather than the clock.
    whole = handle.read_arrow_reader().read_all()
    assert projected.nbytes * 2 <= whole.nbytes

    # The file is unchanged: it still stores all four.
    assert len(handle.read_arrow_field().data_type) == 4
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
    handle.overwriteArrowTable(
      new arrow.Table({
        id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
        symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
        price: arrow.vectorFromArray([1.5, 2.5], new arrow.Float64()),
        venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
      }),
    )

    // Two of the four columns, declared as this read's schema.
    const wanted = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('price: float64')],
      { nullable: false },
    )

    const options = handle.recordOptions().withField(wanted)
    const projected = handle.readArrowReader(options).intoTable()
    assert.equal(projected.numCols, 2)
    assert.deepEqual(projected.schema.fields.map((child) => child.name), ['id', 'price'])

    // The file is unchanged: it still stores all four.
    assert.equal(handle.readArrowField().dataType.length, 4)

    fs.rmSync(root, { recursive: true, force: true })
    ```

The `field` argument to `parquet::read_batch_reader` is a column pushdown and nothing else. A
non-null struct root naming a subset of the stored columns becomes a Parquet `ProjectionMask` over
the file's root columns, which is the format's own way of not reading a column: the chunks it leaves out are never
located, decompressed, or decoded. This is the encoding where a projection genuinely moves less data,
because a Parquet column chunk is separately addressable while an [Arrow IPC](ipc.md) record batch is
one contiguous message.

The mask is built from roots rather than leaves, so a nested column comes along with its whole
subtree. A root naming every stored column, or naming one the file does not store, reads everything:
a mask can only drop columns, never invent them. The selection keeps the stored order and the stored
types, so a caller wanting a different shape casts afterwards.

## Options

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOMedia};
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
        .with_batch_size(256)
        .with_root_name("trade");

    assert_eq!(options.max_row_group_size, 4_096);
    assert_eq!(
        options.key_value_metadata,
        [("iceberg.schema-id".to_owned(), "7".to_owned())]
    );
    assert_eq!(options.batch_size(), Some(256));
    assert_eq!(options.root_name(), "trade");
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

    // batch_size bounds the reader, so no batch holds all 1,000 rows.
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
    options.batch_size = 256
    options.root_name = "trade"

    assert options.max_row_group_size == 4_096
    assert options.key_value_metadata == {"iceberg.schema-id": "7"}
    assert options.batch_size == 256
    assert options.root_name == "trade"
    assert not options.safe

    handle.overwrite_arrow_batch(batch, options=options)

    # batch_size bounds the reader, so no batch holds all 1,000 rows.
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
      .withBatchSize(256)
      .withRootName('trade')

    assert.equal(options.maxRowGroupSize, 4_096)
    assert.deepEqual(options.keyValueMetadata, [{ key: 'iceberg.schema-id', value: '7' }])
    assert.equal(options.batchSize, 256)
    assert.equal(options.rootName, 'trade')
    assert.equal(options.safe, false)

    const ids = Array.from({ length: 1_000 }, (_, index) => BigInt(index))
    handle.overwriteArrowTable(
      new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
      options,
    )

    // batchSize bounds the reader, so no batch holds all 1,000 rows.
    const counts = [...handle.readArrowReader(options)].map((batch) => batch.numRows)
    assert.equal(counts.reduce((total, count) => total + count, 0), 1_000)
    assert.ok(counts.every((count) => count <= 256), counts.join())

    // The root name names the Field recovered from the footer.
    assert.equal(handle.readArrowField(options).name, 'trade')

    fs.rmSync(root, { recursive: true, force: true })
    ```

`ParquetOptions` adds three settings of its own: `compression`, applied to pages inside the file;
`max_row_group_size`, the row bound that decides how many row groups the file gets; and
`key_value_metadata`, entries written into the footer next to the ones the writer adds itself. The
rest are the flat shared fields every record encoding stores under the same names, reached through
[`IORecordOptions`](generic.md): `field`, `root_name`, `safe`, `batch_size`, `max_row_size`,
`max_byte_size`, `commit_row_size`, `level`, `merge_by_names`, `select_by_names`, and
`filter_partitions`.

`level` is the one that does nothing here. It is the compression level of a declared content coding,
and Parquet has no outer coding to apply it to; `compression` is the setting that decides how the
file compresses.

`Parquet::with_options` replaces the whole set, while `with_field` and `with_root_name` reach
through to the two most commonly changed. A declared field short-circuits `field` - it is
returned as it was given, without reading the file - and `root_name` only names a root recovered
from the footer when no schema was declared.

## Compression

!!! note "All three"
    Page compression is a `parquet` crate value, so the bindings name it as the
    text the format's own parser accepts - `zstd(3)`, `snappy`, `uncompressed`.
    Whichever runtime wrote a file, every other one reads it, because the footer
    records the codec.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use parquet::basic::Compression;
    use yggdryl::arrow;
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::{Buffer, IOBase, IOMedia};
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let ids: Vec<i64> = (0..4_000).collect();
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
                    .with_batch_size(batch.num_rows()),
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
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    rows = 4_000
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
        options.batch_size = rows
        handle.overwrite_arrow_table(table, options=options)

        # Nothing on the read side names the compression: the footer records it.
        read = handle.read_arrow_reader(options=options).read_all()
        assert read.num_rows == rows, compression
        sizes.append(handle.size)

    assert sizes[0] > sizes[1] and sizes[0] > sizes[2], sizes
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
    const ids = Array.from({ length: 4_000 }, (_, index) => BigInt(index))
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
        .withBatchSize(table.numRows)
      handle.overwriteArrowTable(table, options)

      // Nothing on the read side names the compression: the footer records it.
      const read = handle.readArrowReader(options).intoTable()
      assert.equal(read.numRows, 4_000, compression)
      sizes.push(handle.size)
    }

    assert.ok(sizes[0] > sizes[1] && sizes[0] > sizes[2], sizes.join())

    fs.rmSync(root, { recursive: true, force: true })
    ```

Compression is a write setting and never a read one. The codec each column chunk was written with is
recorded in the file's own metadata, so a reader recovers it from the footer and the same
`read_arrow_reader` call decodes any of these files. The default is Zstandard at its default level, with
1,048,576-row groups.

## Coded handles are rejected

=== "Rust"

    ```rust
    use arrow_array::RecordBatch;
    use yggdryl::arrow;
    use yggdryl::io::{Buffer, IOBase, IOMedia};
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, Url};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // The name declares gzip over the Parquet file.
    let url = Url::from_str("file:///trades.parquet.gz")?;
    let mut media = Parquet::new(Buffer::new().with_media_type(url.media_type()));

    let empty = arrow::batch_reader(field.into_arrow_schema()?, std::iter::empty::<RecordBatch>());
    let options = media.record_options()?;
    let message = media
        .overwrite_arrow_reader(empty, &options)
        .unwrap_err()
        .to_string();
    assert!(message.contains("parquet compresses"), "{message}");
    assert!(message.contains("ParquetOptions::compression"), "{message}");

    // Nothing was published.
    assert!(media.handle().is_empty());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pytest

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    # The name declares gzip over the Parquet file.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet.gz")

    with pytest.raises(ValueError, match="parquet compresses"):
        handle.overwrite_arrow_batch(pa.record_batch({"id": [1]}, schema=schema))

    # Nothing was published.
    assert handle.size == 0
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

    // The name declares gzip over the Parquet file.
    const handle = new IOBase(path.join(root, 'trades.parquet.gz'))

    assert.throws(
      () =>
        handle.overwriteArrowTable(
          new arrow.Table({ id: arrow.vectorFromArray([1n], new arrow.Int64()) }),
        ),
      /parquet compresses/,
    )

    // Nothing was published.
    assert.equal(handle.size, 0)

    fs.rmSync(root, { recursive: true, force: true })
    ```

Every other encoding treats the content coding as the handle's business: name a file
`trades.arrows.gz` and [ipc.md](ipc.md) writes an Arrow stream through gzip without being told. This
module is the exception. Parquet is a footer-first container that compresses its own pages, and a
coding wrapped around the whole file moves the footer out of reach - the result is bytes no Parquet
reader can open. So a handle whose media type declares any coding other than identity is rejected on
both reads and writes, and the error names `ParquetOptions::compression` as the setting that was
meant instead. The rejection happens before anything is encoded, so a refused write leaves the
handle untouched.

## Field identifiers

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::io::{Buffer, IOMedia};
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id").with_parquet_field_id(1),
        DataType::Utf8.nullable_field("symbol").with_parquet_field_id(2),
    ])?
    .required_field("row");

    let arrow_schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(StringArray::from(vec![Some("AAPL")])),
        ],
    )?;

    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;

    // The ids went into the file, so the Arrow schema carries them back.
    let schema = media.read_arrow_schema()?;
    assert_eq!(
        schema.field(0).metadata().get("PARQUET:field_id"),
        Some(&"1".to_owned())
    );

    // And the recovered Field answers by id rather than by position.
    let recovered = media.read_arrow_field(&options)?;
    assert_eq!(recovered.fields()[0].parquet_field_id()?, Some(1));
    assert_eq!(recovered.fields()[1].parquet_field_id()?, Some(2));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False, metadata={"PARQUET:field_id": "1"}),
        pa.field("symbol", pa.string(), metadata={"PARQUET:field_id": "2"}),
    ])
    batch = pa.record_batch({"id": [1], "symbol": ["AAPL"]}, schema=schema)

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
    handle.overwrite_arrow_batch(batch)

    # The ids went into the file, so the recovered Field answers by id rather
    # than by position.
    recovered = handle.read_arrow_field()
    assert [child.parquet_field_id for child in recovered.data_type] == [1, 2]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    // Arrow JS carries the identifiers the same way Arrow does anywhere else:
    // as field metadata under the exact `PARQUET:field_id` key.
    const rows = new arrow.Table({
      id: arrow.vectorFromArray([1n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL'], new arrow.Utf8()),
    })
    const schema = new arrow.Schema(
      rows.schema.fields.map(
        (child, index) =>
          new arrow.Field(
            child.name,
            child.type,
            child.nullable,
            new Map([['PARQUET:field_id', String(index + 1)]]),
          ),
      ),
    )

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteArrowTable(new arrow.Table(schema, rows.batches[0].data))

    // The ids went into the file, so the recovered Field answers by id rather
    // than by position.
    const recovered = handle.readArrowField()
    assert.deepEqual([...recovered.dataType].map((child) => child.parquetFieldId), [1, 2])
    assert.equal(recovered.dataType.at(0).get('PARQUET:field_id'), '1')

    fs.rmSync(root, { recursive: true, force: true })
    ```

[`Field::with_parquet_field_id`](field.md) stores an identifier under the `PARQUET:field_id` metadata key, which is
exactly the key the Parquet writer reads when it assigns ids in the file's own schema. Projecting the
root to Arrow before building the write's reader is what carries them across, and reading reverses
it. That round trip is
the whole reason a downstream [Iceberg](iceberg.md) or Delta layer can resolve a column after it has
been renamed or moved: the id is in the data file, not just in the catalog.

## Footer statistics

The inferred handle methods validate that the leaf is Parquet, range-read its footer, and decode no
rows. Rust receives the typed `FileStatistics`; Python and JavaScript receive the same shape through
the shared `Scalar` conversion, so integers, byte bounds, nulls, lists, and records become native
language values without binding-side DTO logic.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::io::{Buffer, IOMedia};
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, MimeType, Scalar};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?.required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None, Some("MSFT"), None])),
        ],
    )?;
    let mut media = Parquet::new(
        Buffer::new().with_media_type(MimeType::PARQUET.into()),
    ).with_options(
        ParquetOptions::new()
            .with_max_row_group_size(2)
            .with_key_value("writer", "rust"),
    );
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    let statistics = IOMedia::read_parquet_statistics(&media)?;
    assert_eq!(statistics.num_rows, 4);
    assert_eq!(statistics.row_groups.len(), 2);
    assert_eq!(statistics.null_count("symbol"), Some(2));
    let native = Scalar::from(statistics);
    assert_eq!(native.get_key_str("num_rows").and_then(Scalar::as_i64), Some(4));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    scratch = tempfile.TemporaryDirectory()
    handle = IOBase(pathlib.Path(scratch.name) / "trades.parquet")
    options = handle.record_options()
    options.max_row_group_size = 2
    options.key_value_metadata = {"writer": "python"}
    handle.overwrite_arrow_table(pa.table({"id": [1, 2, 3, 4]}), options=options)

    statistics = handle.read_parquet_statistics()
    assert statistics["num_rows"] == 4
    assert len(statistics["row_groups"]) == 2
    assert next(
        entry for entry in statistics["key_value_metadata"] if entry["key"] == "writer"
    ) == {
        "key": "writer",
        "value": "python",
    }
    assert isinstance(statistics["row_groups"][0]["columns"][0]["min_bytes"], bytes)
    scratch.cleanup()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-parquet-stats-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    const options = handle
      .recordOptions()
      .withMaxRowGroupSize(2)
      .withKeyValue('writer', 'javascript')
    handle.overwriteArrowTable(arrow.tableFromArrays({ id: [1, 2, 3, 4] }), options)

    const statistics = handle.readParquetStatistics()
    assert.equal(statistics.num_rows, 4)
    assert.equal(statistics.row_groups.length, 2)
    assert.deepEqual(statistics.key_value_metadata.find(({ key }) => key === 'writer'), {
      key: 'writer',
      value: 'javascript',
    })
    assert.ok(Buffer.isBuffer(statistics.row_groups[0].columns[0].min_bytes))
    fs.rmSync(root, { recursive: true, force: true })
    ```

`FileStatistics` carries the whole-file row count, writer, ordered footer key/value entries, and
row groups in file order. A row group carries counts, sizes, an optional split offset, and one column
entry per leaf path (`address.zip` for a nested leaf). Key/value metadata stays an entry list because
Parquet permits duplicate keys.

Bounds and null counts are optional because they are optional in the format: a writer records them
per column chunk, and `min_bytes` and `max_bytes` are the encoded values as Parquet stored them, not
decoded scalars. `null_count` sums one column's counts across row groups and returns `None` when no
row group recorded any, which is what distinguishes "no nulls" from "nobody counted".

Rust's `null_count` and `split_offsets` aggregate the same footer data Iceberg manifests use to skip
files. In every language `min_bytes` and `max_bytes` remain Parquet's encoded bounds, not guessed
scalars; missing bounds and counts remain null rather than becoming zero.

A local release boundary spot-check measured footer-to-native-record conversion at 504 us in
Python for 65,536 rows and 728 us in JavaScript for 10,000 rows. The fixtures differ, so these are
per-runtime regression anchors, not a language comparison. Regenerate with the
`parquet read statistics` filter in `python/benchmarks/records_io.py` and
`records/read_parquet_statistics` in `npm run --prefix node bench:records`.

## Geospatial and variant columns

Footer geospatial data crosses with the rest of `read_parquet_statistics`. A fresh projected scan is
`read_parquet_geospatial_statistics(column)` in Python and
`readParquetGeospatialStatistics(column)` in JavaScript; both return native records with
`bounding_box` and `geometry_types` through the same shared `Scalar` shape.

A column whose schema declares [geometry or geography](datatype.md) writes Parquet's own `GEOMETRY`
or `GEOGRAPHY` logical type over `BYTE_ARRAY` WKB, from the schema's own declaration: the CRS and,
for a geography, the edge algorithm ride along, and the defaults - `OGC:CRS84`, `spherical` - fold
to the format's absent spellings, so a bare declaration writes the bare logical type. A variant
field writes its metadata/value storage struct with the `VARIANT` logical type attached, at the
schema level only: a variant *value* cannot cross an Arrow array boundary yet - the variant binary
encoding lands with the Iceberg v3 layer - so variant columns stay schema-level until it does.

A geospatial column's sort order is undefined, so the writer never records min/max value bounds for
it - a bound would be a lie - while sibling columns keep theirs, and a min/max a foreign writer
recorded anyway is ignored on read rather than surfaced. What a geometry records instead is the
format's own geospatial statistics: the WKB bounding box and the sorted ISO geometry type codes
present, in the footer, readable from `ColumnStatistics::geospatial` and recomputable by scanning
the stored WKB through Rust's `read_geospatial_statistics` (the inferred handle methods above in the
bindings), which also answers for files whose writer recorded none. A geography records no box at
all: its bounds are edge-algorithm-aware, and a planar fold of the vertices would under-cover them.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{BinaryArray, Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::io::{Buffer, IOMedia};
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    fn wkb_point(x: f64, y: f64) -> Vec<u8> {
        let mut bytes = vec![1u8];
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes
    }

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::geometry(None)?.nullable_field("shape"),
    ])?
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(BinaryArray::from_opt_vec(vec![
                Some(&wkb_point(1.0, 2.0)[..]),
                None,
                Some(&wkb_point(-3.0, 7.0)[..]),
            ])),
        ],
    )?;
    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    let statistics = media.read_statistics()?;
    let columns = &statistics.row_groups[0].columns;
    let id = columns.iter().find(|column| column.path == "id").unwrap();
    let shape = columns.iter().find(|column| column.path == "shape").unwrap();
    assert!(id.min_bytes.is_some() && id.max_bytes.is_some());
    assert!(shape.min_bytes.is_none() && shape.max_bytes.is_none());
    assert_eq!(shape.null_count, Some(1));

    let geospatial = shape.geospatial.as_ref().unwrap();
    let bounds = geospatial.bounding_box.unwrap();
    assert_eq!(
        (bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax),
        (-3.0, 1.0, 2.0, 7.0)
    );
    assert_eq!(geospatial.geometry_types, vec![1]);
    assert_eq!(
        IOMedia::read_parquet_geospatial_statistics(&media, "shape")?,
        *geospatial,
    );
    ```

=== "Python"

    ```python
    import pathlib
    import struct
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    def point(x: float, y: float) -> bytes:
        return b"\x01\x01\x00\x00\x00" + struct.pack("<dd", x, y)

    schema = pa.schema([
        pa.field(
            "shape",
            pa.binary(),
            metadata={
                b"ARROW:extension:name": b"geoarrow.wkb",
                b"ARROW:extension:metadata": b'{"crs":"OGC:CRS84"}',
            },
        )
    ])
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "shapes.parquet")
    handle.overwrite_arrow_table(
        pa.table({"shape": [point(1, 2), None, point(-3, 7)]}, schema=schema)
    )

    scanned = handle.read_parquet_geospatial_statistics("shape")
    footer = handle.read_parquet_statistics()["row_groups"][0]["columns"][0][
        "geospatial"
    ]
    assert scanned == footer
    assert scanned["geometry_types"] == [1]
    assert scanned["bounding_box"]["xmin"] == -3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const point = (x, y) => {
      const bytes = Buffer.allocUnsafe(21)
      bytes.writeUInt8(1, 0)
      bytes.writeUInt32LE(1, 1)
      bytes.writeDoubleLE(x, 5)
      bytes.writeDoubleLE(y, 13)
      return bytes
    }

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-geo-'))
    const handle = new IOBase(path.join(root, 'shapes.parquet'))
    handle.overwriteArrowTable(new arrow.Table({
      shape: arrow.vectorFromArray(
        [point(1, 2), null, point(-3, 7)],
        new arrow.Binary(),
      ),
    }))

    const scanned = handle.readParquetGeospatialStatistics('shape')
    assert.deepEqual(scanned.geometry_types, [1])
    assert.equal(scanned.bounding_box.xmin, -3)
    assert.equal(scanned.bounding_box.ymax, 7)
    fs.rmSync(root, { recursive: true, force: true })
    ```

One read-side limit is named rather than hidden: a *foreign* file whose columns carry
`GEOMETRY`/`GEOGRAPHY`/`VARIANT` surfaces plain `Binary`/`Struct` Arrow types without extension
metadata, because the pinned parquet crate only maps those logical types to Arrow extensions behind
crate features that pull new dependencies; files written here round-trip their extension identity
through the embedded Arrow schema. And GeoArrow's own documents say the specification is not
finalized, so the `geoarrow.wkb` spelling the writer reads is revisitable if it changes.

The projected WKB-to-native-record boundary measured 2.64 ms in Python for 8,192 rows and
3.26 ms in JavaScript for 10,000 rows in the same local release spot-check. Regenerate with the
`parquet read geospatial stats` filter in `python/benchmarks/records_io.py` and
`records/read_parquet_geospatial_statistics` in `npm run --prefix node bench:records`.

## The handle underneath

!!! note "Rust encoding seam"
    The named `Parquet<H>` wrapper and free functions are Rust's typed encoding
    seam. Python and JavaScript infer and retain the same wrapper inside an
    opened `IOBase`; callers keep one generic handle surface.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::io::{Buffer, IOBase, IOMedia};
use yggdryl::parquet::{self, Parquet, ParquetOptions};
use yggdryl::{DataType, MimeType};

let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = field.into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

// The free functions take a handle and options; nothing is bound.
let options = ParquetOptions::new();
let mut handle = Buffer::new().with_media_type(MimeType::PARQUET.into());
parquet::overwrite_arrow_reader(
    &mut handle,
    arrow::batch_reader(arrow_schema, [batch]),
    &options,
)?;

assert_eq!(parquet::read_arrow_schema(&handle)?.fields().len(), 1);
assert_eq!(parquet::read_field(&handle, &options)?.name(), "row");
assert_eq!(parquet::read_batch_reader(&handle, None, &options)?.count(), 1);
assert_eq!(parquet::read_statistics(&handle)?.num_rows, 2);

// A Parquet is also the bytes it encodes, magic bytes included.
let mut media = Parquet::new(handle);
assert_eq!(media.read_range_bytes(0, 4)?, *b"PAR1");

// open caches the footer, close releases it.
assert!(!media.opened());
media.open()?;
assert!(media.opened());
assert_eq!(media.read_statistics()?.num_rows, 2);
assert_eq!(media.row_size()?, 2);
assert_eq!(media.column_size()?, 1);
media.close()?;
assert!(!media.opened());
```

`Parquet<H>` is itself an [`IOBase`](io.md) over the handle it owns, so the encoded file is reachable
without unwrapping anything - to copy it, upload it, or hand it to another reader. It forwards every
byte method to the handle and keeps `open`, `opened`, and `close` for itself: `open` parses the
footer once and caches it, so repeated statistics reads do not re-parse it, `close` drops it, and any
write invalidates it.

When the encoding is decided at run time rather than written into the type,
[`Media::parquet`](generic.md) names this variant and [`IOMedia::record_options`](io.md) derives
`ParquetOptions` from a handle's own media type, so a file named `trades.parquet` is read as Parquet
without a format argument.

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::io::{Buffer, IOMedia};
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // Nothing has been written, so there is nothing to read.
    let empty = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
        .with_field(field.clone());
    let options = empty.record_options()?;
    let reader = empty.read_arrow_reader(&options)?;
    assert_eq!(reader.schema().fields().len(), 1);
    assert_eq!(reader.count(), 0);

    // An empty write still publishes a readable file with the schema in its footer.
    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    let options = media.record_options()?;
    media.overwrite_arrow_reader(
        arrow::batch_reader(
            field.into_arrow_schema()?,
            std::iter::empty::<RecordBatch>(),
        ),
        &options,
    )?;
    assert_eq!(media.read_arrow_reader(&options)?.count(), 0);
    assert_eq!(media.read_arrow_schema()?.fields().len(), 1);
    assert_eq!(media.read_statistics()?.num_rows, 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    root = pathlib.Path(tempfile.mkdtemp())

    # Nothing has been written, so there is nothing to read.
    empty = IOBase(root / "absent.parquet")
    assert empty.read_arrow_reader().read_all().num_rows == 0

    # An empty write still publishes a readable file with the schema in its footer.
    handle = IOBase(root / "written.parquet")
    handle.overwrite_arrow_table(pa.Table.from_batches([], schema=schema))
    assert handle.size > 0
    assert handle.read_arrow_reader().read_all().num_rows == 0
    assert len(handle.read_arrow_field().data_type) == 1
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

    // Nothing has been written, so there is nothing to read.
    const empty = new IOBase(path.join(root, 'absent.parquet'))
    assert.equal(empty.readArrowReader().intoTable().numRows, 0)

    // An empty write still publishes a readable file with the schema in its footer.
    const schema = new arrow.Schema([new arrow.Field('id', new arrow.Int64(), true)])
    const handle = new IOBase(path.join(root, 'written.parquet'))
    handle.overwriteArrowTable(new arrow.Table(schema))
    assert.ok(handle.size > 0)
    assert.equal(handle.readArrowReader().intoTable().numRows, 0)
    assert.equal(handle.readArrowField().dataType.length, 1)

    fs.rmSync(root, { recursive: true, force: true })
    ```

An absent file holds no batches rather than failing on a missing footer, which is the laziness
contract every handle follows: constructing touches nothing, reading something absent yields nothing,
writing creates. With no file to read a schema from, the declared one is what the empty reader
reports. Writing no batches is a different thing entirely - it publishes a real file, so the schema
and the statistics are there with no rows behind them.

## Against PyArrow

`python/benchmarks/records_io.py` carries PyArrow Parquet baselines over the same rows.
`records_io.py --min-time 0.1 --repeat 3`, one containerized x86_64 Linux run, 65,536 rows,
4 columns, 8 batches:

```text
parquet write reader             6.932 ms    9.5M rows/s
PyArrow parquet write baseline   6.495 ms   10.1M rows/s
parquet read whole               2.620 ms   25.0M rows/s
PyArrow parquet read baseline    2.195 ms   29.9M rows/s
```

Both directions sit within ~15% of PyArrow's own writer and reader - the encoding dominates and
both sides drive the same `parquet` machinery. The [ipc](ipc.md) page carries that encoding's
rows from the same run.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/parquet.ipynb){ download },
[Python](notebooks/python/parquet.ipynb){ download },
[JavaScript](notebooks/javascript/parquet.ipynb){ download }.

<!-- /notebooks -->
