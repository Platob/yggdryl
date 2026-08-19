# Apache Parquet

Read and write Apache Parquet files, and their footer statistics, over any handle.

!!! note "All three"
    Python and JavaScript reach the encoding through [`IOBase`](io.md)'s record
    methods, which cover reading, writing, column pushdown, row groups, and
    footer metadata; the statistics reader and the stateful `Parquet` wrapper
    stay in Rust, and each section below says so.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::io::Buffer;
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, Url};

    // A non-null struct Field is the schema.
    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let arrow_schema = field.to_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None, Some("MSFT")])),
        ],
    )?;

    let url = Url::from_str("file:///trades.parquet")?;
    let mut media = Parquet::new(Buffer::new().with_media_type(url.media_type()));
    media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch.clone()]))?;

    // Reading streams: one batch at a time, never one materialized table.
    let read = media.read_batch_reader(None)?.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(read, [batch]);
    assert_eq!(read[0].num_rows(), 3);
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
    batch = pa.record_batch(
        {"id": [1, 2, 3], "symbol": ["AAPL", None, "MSFT"]}, schema=schema
    )

    # The name says Parquet, so no call names an encoding.
    path = pathlib.Path(tempfile.mkdtemp()) / "trades.parquet"
    with IOBase(path) as handle:
        handle.write_arrow_batch_reader(batch)

        # Reading streams: one batch at a time, never one materialized table.
        assert handle.read_arrow_batch_reader().read_all().num_rows == 3

    # The scope published the file at its exact length, so PyArrow's own reader
    # finds the footer where the format says it is.
    assert pq.read_table(path) == pa.Table.from_batches([batch])
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
      id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null, 'MSFT'], new arrow.Utf8()),
    })

    // The name says Parquet, so no call names an encoding.
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.writeArrowBatchReader(table)

    // Reading streams: one batch at a time, never one materialized table.
    assert.equal(handle.readArrowBatchReader().toTable().numRows, 3)
    assert.deepEqual([...handle.readBytes().subarray(0, 4)], [...Buffer.from('PAR1')])

    fs.rmSync(root, { recursive: true, force: true })
    ```

Everything on this page is behind the non-default `parquet` feature. The codec is version-locked to
the pinned Arrow release and pulls in a thrift and compression stack a schema-only consumer never
touches, so it is opted into rather than carried. Without it the module does not exist, and
[`RecordOptions::for_mime_type`](generic.md) reports `application/vnd.apache.parquet` as an encoding
this build does not implement instead of guessing one.

`Parquet<H>` binds one file to one handle and owns every setting, so neither is passed again at each
call. Streaming is the only shape: `Parquet::write_batch_reader` consumes an
[`arrow::BatchReader`](arrow.md) and replaces the file, and `Parquet::read_batch_reader` returns one.
`arrow::batch_reader` is the constructor that turns batches a caller already has into one. There is
no row-level read or write anywhere here - a batch is the unit - and every batch a write pulls must
match the schema its reader declares, which is checked per batch and reported with the index that
disagreed.

`Parquet::read_batch_reader` takes the columns to keep, `read_schema` returns the Arrow schema, `read_field`
and `schema` return the canonical non-null struct root [`Field`](field.md), and `read_statistics`
returns the footer. Each of these exists as a free function too - `parquet::read_schema`,
`read_field`, `read_batch_reader`, `write_batch_reader`, `read_statistics` - taking the handle
explicitly, and a `&ParquetOptions` where the settings matter.

Both bindings call the same operations on the handle itself - `read_arrow_batch_reader` and
`write_arrow_batch_reader` - so a file this large never becomes a table on either side unless a caller asks
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
    use yggdryl::io::Buffer;
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let stored = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::Utf8.required_field("venue"),
    ])?
    .required_field("row");
    let arrow_schema = stored.to_arrow_schema()?;

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
    media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;

    // Two of the four columns, named by a root Field of its own.
    let wanted = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])?
    .required_field("row");

    let projected = media.read_batch_reader(Some(&wanted))?;
    assert_eq!(projected.schema().fields().len(), 2);
    let read = projected.collect::<Result<Vec<_>, _>>()?;
    assert_eq!(read[0].num_columns(), 2);

    // The file is unchanged: it still stores all four.
    assert_eq!(media.read_schema()?.fields().len(), 4);
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
    handle.write_arrow_batch_reader(batch)

    # Two of the four columns, declared as this read's schema - a single
    # setting is its own keyword, no options object needed.
    projected = handle.read_arrow_batch_reader(
        schema=pa.schema([
            pa.field("id", pa.int64(), nullable=False),
            pa.field("price", pa.float64(), nullable=False),
        ])
    ).read_all()
    assert projected.column_names == ["id", "price"]

    # Less is read, and the bytes say so rather than the clock.
    whole = handle.read_arrow_batch_reader().read_all()
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
    handle.writeArrowBatchReader(
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

    const options = handle.recordOptions().withSchema(wanted)
    const projected = handle.readArrowBatchReader(options).toTable()
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
    use yggdryl::io::Buffer;
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, Level, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
    let arrow_schema = field.to_arrow_schema()?;
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
    media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;

    // batch_size bounds the reader, so no batch holds all 1,000 rows.
    let rows: Vec<usize> = media
        .read_batch_reader(None)?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .map(arrow_array::RecordBatch::num_rows)
        .collect();
    assert_eq!(rows.iter().sum::<usize>(), 1_000);
    assert!(rows.iter().all(|count| *count <= 256), "{rows:?}");

    // The root name names the Field recovered from the footer.
    assert_eq!(media.read_field()?.name(), "trade");

    // A declared schema is returned as-is, so an empty handle answers without a footer.
    let declared =
        Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into())).with_schema(field.clone());
    assert_eq!(declared.read_field()?, field);
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

    handle.write_arrow_batch_reader(batch, options=options)

    # batch_size bounds the reader, so no batch holds all 1,000 rows.
    counts = [part.num_rows for part in handle.read_arrow_batch_reader(options=options)]
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
    handle.writeArrowBatchReader(
      new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
      options,
    )

    // batchSize bounds the reader, so no batch holds all 1,000 rows.
    const counts = [...handle.readArrowBatchReader(options)].map((batch) => batch.numRows)
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
[`IORecordOptions`](generic.md): `schema`, `root_name`, `safe`, `batch_size`, and `level`.

`level` is the one that does nothing here. It is the compression level of a declared content coding,
and Parquet has no outer coding to apply it to; `compression` is the setting that decides how the
file compresses.

`Parquet::with_options` replaces the whole set, while `with_schema` and `with_root_name` reach
through to the two most commonly changed. A declared schema short-circuits `read_field` - it is
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
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::parquet::{Parquet, ParquetOptions};
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let ids: Vec<i64> = (0..4_000).collect();
    let symbols: Vec<Option<&str>> = ids.iter().map(|_| Some("AAPL")).collect();
    let arrow_schema = field.to_arrow_schema()?;
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
        media.write_batch_reader(arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]))?;

        // Nothing on the read side names the compression: the footer records it.
        let read = media.read_batch_reader(None)?.collect::<Result<Vec<_>, _>>()?;
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
        handle.write_arrow_batch_reader(table, options=options)

        # Nothing on the read side names the compression: the footer records it.
        read = handle.read_arrow_batch_reader(options=options).read_all()
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
      handle.writeArrowBatchReader(table, options)

      // Nothing on the read side names the compression: the footer records it.
      const read = handle.readArrowBatchReader(options).toTable()
      assert.equal(read.numRows, 4_000, compression)
      sizes.push(handle.size)
    }

    assert.ok(sizes[0] > sizes[1] && sizes[0] > sizes[2], sizes.join())

    fs.rmSync(root, { recursive: true, force: true })
    ```

Compression is a write setting and never a read one. The codec each column chunk was written with is
recorded in the file's own metadata, so a reader recovers it from the footer and the same
`read_batch_reader` call decodes any of these files. The default is Zstandard at its default level, with
1,048,576-row groups.

## Coded handles are rejected

=== "Rust"

    ```rust
    use arrow_array::RecordBatch;
    use yggdryl::arrow;
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, Url};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // The name declares gzip over the Parquet file.
    let url = Url::from_str("file:///trades.parquet.gz")?;
    let mut media = Parquet::new(Buffer::new().with_media_type(url.media_type()));

    let empty = arrow::batch_reader(field.to_arrow_schema()?, std::iter::empty::<RecordBatch>());
    let message = media.write_batch_reader(empty).unwrap_err().to_string();
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
        handle.write_arrow_batch_reader(pa.record_batch({"id": [1]}, schema=schema))

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
        handle.writeArrowBatchReader(
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
    use yggdryl::io::Buffer;
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([
        DataType::Int64.required_field("id").with_parquet_field_id(1),
        DataType::Utf8.nullable_field("symbol").with_parquet_field_id(2),
    ])?
    .required_field("row");

    let arrow_schema = field.to_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1])),
            Arc::new(StringArray::from(vec![Some("AAPL")])),
        ],
    )?;

    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;

    // The ids went into the file, so the Arrow schema carries them back.
    let schema = media.read_schema()?;
    assert_eq!(
        schema.field(0).metadata().get("PARQUET:field_id"),
        Some(&"1".to_owned())
    );

    // And the recovered Field answers by id rather than by position.
    let recovered = media.read_field()?;
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
    handle.write_arrow_batch_reader(batch)

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
    handle.writeArrowBatchReader(new arrow.Table(schema, rows.batches[0].data))

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

!!! note "Rust only"
    The bindings read a file's schema and rows but not its footer statistics
    yet.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::arrow;
use yggdryl::io::Buffer;
use yggdryl::parquet::{Parquet, ParquetOptions};
use yggdryl::{DataType, MimeType};

let field = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::Utf8.nullable_field("symbol"),
])?
.required_field("row");

let ids: Vec<i64> = (0..2_048).collect();
let symbols: Vec<Option<&str>> = ids
    .iter()
    .map(|index| (index % 2 == 0).then_some("AAPL"))
    .collect();
let arrow_schema = field.to_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![
        Arc::new(Int64Array::from(ids)),
        Arc::new(StringArray::from(symbols)),
    ],
)?;

let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into())).with_options(
    ParquetOptions::new()
        .with_max_row_group_size(512)
        .with_key_value("iceberg.schema-id", "7"),
);
media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;

let statistics = media.read_statistics()?;
assert_eq!(statistics.num_rows, 2_048);
assert_eq!(statistics.row_groups.len(), 4);
assert!(statistics.created_by.is_some());
assert!(
    statistics
        .key_value_metadata
        .iter()
        .any(|(key, value)| key == "iceberg.schema-id" && value == "7"),
    "{:?}",
    statistics.key_value_metadata
);

// Null counts are summed over every row group; an unknown path is None, not zero.
assert_eq!(statistics.null_count("symbol"), Some(1_024));
assert_eq!(statistics.null_count("id"), Some(0));
assert_eq!(statistics.null_count("absent"), None);

// One offset per row group, ascending.
let offsets = statistics.split_offsets();
assert_eq!(offsets.len(), 4);
assert!(offsets.windows(2).all(|pair| pair[0] < pair[1]), "{offsets:?}");

let first = &statistics.row_groups[0];
assert_eq!(first.num_rows, 512);
assert!(first.compressed_size > 0);
assert!(
    first
        .columns
        .iter()
        .any(|column| column.path == "id" && column.min_bytes.is_some())
);
```

`read_statistics` parses the footer and decodes no rows. `FileStatistics` carries the whole-file row
count, the writer that produced the file, the footer's key/value entries - the ones written through
`with_key_value` alongside the ones the writer adds itself - and one `RowGroupStatistics` per row
group in file order. Each of those carries its row count, its total compressed size, its byte offset
in the file when the writer recorded one, and one `ColumnStatistics` per leaf column - compressed and
uncompressed size, null count, bounds - keyed by dotted path, so a nested column appears as
`address.zip`.

Bounds and null counts are optional because they are optional in the format: a writer records them
per column chunk, and `min_bytes` and `max_bytes` are the encoded values as Parquet stored them, not
decoded scalars. `null_count` sums one column's counts across row groups and returns `None` when no
row group recorded any, which is what distinguishes "no nulls" from "nobody counted".

`split_offsets` exists because [Iceberg](iceberg.md) records exactly that sequence for a data file,
and the per-column null counts and bounds are what a manifest entry stores next to it, so a planner
can skip whole files it never has to open.

## Geospatial and variant columns

!!! note "Rust only"
    The bindings declare [geometry, geography, and variant fields](datatype.md)
    and write them through the same record methods, but the statistics surface
    below is Rust only, like the rest of the footer above.

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
the stored WKB through `read_geospatial_statistics` - which also answers for files whose writer
recorded none. A geography records no box at all: its bounds are edge-algorithm-aware, and a planar
fold of the vertices would under-cover them.

```rust
use std::sync::Arc;

use arrow_array::{BinaryArray, Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::io::Buffer;
use yggdryl::parquet::Parquet;
use yggdryl::{DataType, MimeType};

/// One little-endian ISO WKB point.
fn wkb_point(x: f64, y: f64) -> Vec<u8> {
    let mut bytes = vec![1u8];
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&x.to_le_bytes());
    bytes.extend_from_slice(&y.to_le_bytes());
    bytes
}

// The declaration is the schema's own: `None` fills the `OGC:CRS84` default,
// which folds to Parquet's bare `GEOMETRY` spelling.
let field = DataType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::geometry(None)?.nullable_field("shape"),
])?
.required_field("row");

let arrow_schema = field.to_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
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
media.write_batch_reader(arrow::batch_reader(arrow_schema, [batch]))?;

let statistics = media.read_statistics()?;
let columns = &statistics.row_groups[0].columns;
let id = columns.iter().find(|column| column.path == "id").unwrap();
let shape = columns.iter().find(|column| column.path == "shape").unwrap();

// The sibling keeps its value bounds; the geometry never records any.
assert!(id.min_bytes.is_some() && id.max_bytes.is_some());
assert!(shape.min_bytes.is_none() && shape.max_bytes.is_none());
assert_eq!(shape.null_count, Some(1));

// What a geometry records instead: the WKB bounds and type codes.
let geospatial = shape.geospatial.as_ref().unwrap();
let bounds = geospatial.bounding_box.unwrap();
assert_eq!(
    (bounds.xmin, bounds.xmax, bounds.ymin, bounds.ymax),
    (-3.0, 1.0, 2.0, 7.0)
);
assert_eq!(geospatial.geometry_types, vec![1]); // an XY point is code 1

// A fresh scan of the stored WKB answers the same statistics.
assert_eq!(media.read_geospatial_statistics("shape")?, *geospatial);
```

One read-side limit is named rather than hidden: a *foreign* file whose columns carry
`GEOMETRY`/`GEOGRAPHY`/`VARIANT` surfaces plain `Binary`/`Struct` Arrow types without extension
metadata, because the pinned parquet crate only maps those logical types to Arrow extensions behind
crate features that pull new dependencies; files written here round-trip their extension identity
through the embedded Arrow schema. And GeoArrow's own documents say the specification is not
finalized, so the `geoarrow.wkb` spelling the writer reads is revisitable if it changes.

## The handle underneath

!!! note "Rust only"
    The bindings call the record methods on the handle directly, so there is no
    wrapper to reach through.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::arrow;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::parquet::{self, Parquet, ParquetOptions};
use yggdryl::{DataType, MimeType};

let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");
let arrow_schema = field.to_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![1, 2]))],
)?;

// The free functions take a handle and options; nothing is bound.
let options = ParquetOptions::new();
let mut handle = Buffer::new().with_media_type(MimeType::PARQUET.into());
parquet::write_batch_reader(
    &mut handle,
    arrow::batch_reader(arrow_schema, [batch]),
    &options,
)?;

assert_eq!(parquet::read_schema(&handle)?.fields().len(), 1);
assert_eq!(parquet::read_field(&handle, &options)?.name(), "row");
assert_eq!(parquet::read_batch_reader(&handle, None, &options)?.count(), 1);
assert_eq!(parquet::read_statistics(&handle)?.num_rows, 2);

// A Parquet is also the bytes it encodes, magic bytes included.
let mut media = Parquet::new(handle);
assert_eq!(media.read_range(0, 4)?, *b"PAR1");

// open caches the footer, close releases it.
assert!(!media.opened());
media.open()?;
assert!(media.opened());
assert_eq!(media.read_statistics()?.num_rows, 2);
media.close()?;
assert!(!media.opened());
```

`Parquet<H>` is itself an [`IOBase`](io.md) over the handle it owns, so the encoded file is reachable
without unwrapping anything - to copy it, upload it, or hand it to another reader. It forwards every
byte method to the handle and keeps `open`, `opened`, and `close` for itself: `open` parses the
footer once and caches it, so repeated statistics reads do not re-parse it, `close` drops it, and any
write invalidates it.

Reads currently fetch the handle's bytes whole. Parquet's footer is at the end of the file, so a
partial read has to be a range read against [`IOBase::pread`](io.md); until that path exists this is
buffered rather than pretending otherwise. That is also the bound on what column pushdown saves
today: the projection mask skips locating, decompressing, and decoding the columns it drops, but the
file's bytes still arrive in one piece.

When the encoding is decided at run time rather than written into the type,
[`Media::parquet`](generic.md) names this variant and [`IOBase::record_options`](io.md) derives
`ParquetOptions` from a handle's own media type, so a file named `trades.parquet` is read as Parquet
without a format argument.

=== "Rust"

    ```rust
    use arrow_array::{RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::io::Buffer;
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType};

    let field = DataType::from_fields([DataType::Int64.required_field("id")])?.required_field("row");

    // Nothing has been written, so there is nothing to read.
    let empty = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()))
        .with_schema(field.clone());
    assert_eq!(empty.read_batch_reader(None)?.count(), 0);
    assert_eq!(empty.read_batch_reader(None)?.schema().fields().len(), 1);

    // An empty write still publishes a readable file with the schema in its footer.
    let mut media = Parquet::new(Buffer::new().with_media_type(MimeType::PARQUET.into()));
    media.write_batch_reader(arrow::batch_reader(
        field.to_arrow_schema()?,
        std::iter::empty::<RecordBatch>(),
    ))?;
    assert_eq!(media.read_batch_reader(None)?.count(), 0);
    assert_eq!(media.read_schema()?.fields().len(), 1);
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
    assert empty.read_arrow_batch_reader().read_all().num_rows == 0

    # An empty write still publishes a readable file with the schema in its footer.
    handle = IOBase(root / "written.parquet")
    handle.write_arrow_batch_reader(pa.Table.from_batches([], schema=schema))
    assert handle.size > 0
    assert handle.read_arrow_batch_reader().read_all().num_rows == 0
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
    assert.equal(empty.readArrowBatchReader().toTable().numRows, 0)

    // An empty write still publishes a readable file with the schema in its footer.
    const schema = new arrow.Schema([new arrow.Field('id', new arrow.Int64(), true)])
    const handle = new IOBase(path.join(root, 'written.parquet'))
    handle.writeArrowBatchReader(new arrow.Table(schema))
    assert.ok(handle.size > 0)
    assert.equal(handle.readArrowBatchReader().toTable().numRows, 0)
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
