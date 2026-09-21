# Parquet column pushdown

A read field naming a subset of the stored columns becomes a `ProjectionMask` over root columns, and the chunks it leaves out are never located, decompressed, or decoded.

## Contract

| Item | Behaviour |
| --- | --- |
| Owned | the `field` a read takes, and the options' `select`, turned into a Parquet `ProjectionMask` before the reader builds |
| Cost | the excluded column chunks are never read, so this moves fewer bytes rather than only fewer allocations |
| Granularity | root columns: a nested column comes along whole, because the mask is built from roots, not leaves |
| Shape | the projected columns come back in stored order with stored types; a caller wanting another shape casts afterwards |
| Lazy | the file is untouched - a projection is a read, and the footer still describes every stored column |
| Refused | nothing: a field naming a column the file does not store is never an error - the mask is dropped instead, so every chunk is read and the column comes back all-null |
| Bindings | Python `options.field`, JavaScript `withField`, Rust the `field` on the record options or `parquet::read_batch_reader` |
| Against IPC | an [Arrow IPC](../ipc/index.md) batch body is one message and is still read whole; a Parquet column chunk is separately addressable |

## Use

A non-null struct root naming a subset of the stored columns reads only those chunks. Unlike an IPC batch, a Parquet column chunk has its own byte range in the footer, so the reader can skip it.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Float64Array, Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::IOMedia;
    use yggdryl::holder::Buffer;
    use yggdryl::parquet::Parquet;
    use yggdryl::{DataType, MimeType, StructType};

    let stored = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
        DataType::Float64.required_field("price"),
        DataType::utf8().required_field("venue"),
    ])?)
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
    let wanted = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Float64.required_field("price"),
    ])?)
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

    # A name the file does not store drops the mask instead: every chunk is
    # read, and the column comes back all-null under the declared shape.
    absent = handle.record_options()
    absent.field = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("nope", pa.int64()),
    ])
    invented = handle.read_arrow_reader(options=absent).read_all()
    assert invented.column_names == ["id", "nope"]
    assert invented["nope"].null_count == rows

    # The file is unchanged: it still stores all four.
    assert len(handle.read_arrow_field().dtype) == 4
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
    assert.equal(handle.readArrowField().dtype.length, 4)

    fs.rmSync(root, { recursive: true, force: true })
    ```

| root field | mask |
| --- | --- |
| a subset of the stored columns | only those chunks; a caller wanting another shape casts afterwards |
| a nested column | its whole subtree; the mask is built from roots, not leaves |
| every stored column | no mask: that is the read that already happens |
| one the file lacks | no mask either, so every chunk is read; the result still takes the declared shape, and the unstored column comes back all-null |

## Edges

- read `field` naming every stored column -> no mask: that is the read that already happens.
- read `field` naming a column the file does not store -> no mask either, so every chunk is read and the saving is silently lost; the result still takes the declared shape, and the unstored column comes back all-null.
- projected read -> the reader reports the projected schema, and the skipped chunks are never touched.
- a nested column -> selected whole; the mask cannot name one leaf of a struct.
- the file after a projected read -> unchanged; `read_arrow_schema` and the [footer](footer.md) still describe every stored column.
- a projected write -> not a thing: the three [write intents](write.md) publish the columns the batches carry.

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test parquet -- mod_::internal
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/parquet
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_init.py
    python/.venv/bin/python python/benchmarks/media.py --filter "parquet read subset"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/records.test.js
    YGGDRYL_BENCH_FILTER=records/read_parquet_pushdown npm run --prefix node bench:media
    ```
