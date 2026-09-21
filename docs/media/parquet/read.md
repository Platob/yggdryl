# Apache Parquet reads

Rows out of a Parquet file: native scalars first, then Arrow batches, and the schema reads that touch nothing but the footer.

## Contract

| Item | Behaviour |
| --- | --- |
| Native rows | Python `read_records`, JavaScript `readRecords`; Rust reads Arrow and crosses with `ArrowScalar::into_scalar` |
| Row shape | an ordered `Scalar::Sequence` under the root, or a name-sorted `Scalar::Struct` resolved to that order |
| Batches | `read_arrow_reader` returns an [`arrow::BatchReader`](../../arrow/readers.md), bounded by `batch_row_size` and `batch_byte_size`, never a collected table |
| Schema | `read_arrow_field` returns the canonical non-null struct root [`Field`](../../types/field.md); Rust `read_arrow_schema` returns the stored Arrow schema, field metadata included |
| Declared root | `options.field` is returned as-is, so a declared handle answers without reading a footer |
| Pushdown | a `field` naming a subset of the stored columns is a `ProjectionMask` over root columns: [Pushdown](pushdown.md) |
| Limits | `max_row_size` is a fetch plan as well as a bound: only the leading row groups covering it are read, and the result is still trimmed to the exact count |
| Lazy | an absent resource yields zero batches under the declared schema, not a missing-footer error |
| Refused | a coded handle before a byte is decoded ([Compression](compression.md)); `read_scalar` on a Parquet name, which is a structured-text call |
| Bindings | Python hands back a `pyarrow.RecordBatchReader` over the Arrow C Stream; JavaScript one Arrow JS batch per stream over copied IPC; neither builds a table unasked |

## Use

A row comes back as the value the runtime spells natively - a mapping in Python, a plain object in JavaScript - with no Arrow type in the call. Rust reads Arrow and crosses into the value model in one call.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, Scalar, StructType, Url};

    struct Trade(i64, &'static str);

    impl From<Trade> for Scalar {
        fn from(row: Trade) -> Self {
            Scalar::from_sequence([Scalar::from(row.0), Scalar::from(row.1)])
        }
    }

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
    ])?)
    .required_field("row");
    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.parquet")?.media_type());
    let options = handle.record_options()?.with_field(field.clone());

    handle.overwrite_records([Trade(1, "AAPL"), Trade(2, "MSFT")], &options)?;

    // Rust reads Arrow, then crosses into the value model in one call: a file
    // answers a sequence of ordered row sequences.
    let rows = handle.read_arrow(Some(&options.clone().with_field(field.clone())))?.into_scalar()?;
    let symbols = rows
        .as_sequence()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row.as_sequence().and_then(|row| row[1].as_str()))
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 2);
    assert!(symbols.contains(&"AAPL") && symbols.contains(&"MSFT"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
    handle.overwrite_records([{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": "MSFT"}])

    # Plain dictionaries back, one row at a time.
    rows = list(handle.read_records())
    assert len(rows) == 2
    assert {row["symbol"] for row in rows} == {"AAPL", "MSFT"}
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.parquet'))
    handle.overwriteRecords([{ id: 1n, symbol: 'AAPL' }, { id: 2n, symbol: 'MSFT' }])

    // Plain objects back, one row at a time.
    const rows = [...handle.readRecords()]
    assert.equal(rows.length, 2)
    assert.deepEqual(new Set(rows.map((row) => row.symbol)), new Set(['AAPL', 'MSFT']))

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Rows as Arrow batches

The same rows, without the value model in the middle: `read_arrow_reader` streams batches and `read_arrow_field` names the root it read them under.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, StructType, Url};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), Some("MSFT")])),
        ],
    )?;

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.parquet")?.media_type());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    // A stream of batches, and a root Field read from the footer alone.
    assert_eq!(handle.read_arrow_reader(&options)?.count(), 1);
    assert_eq!(handle.read_arrow_field(&options)?.field_len(), 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}))

    # A pyarrow.RecordBatchReader: collecting it is the caller's choice.
    reader = handle.read_arrow_reader()
    assert reader.read_all().num_rows == 2
    assert len(handle.read_arrow_field().dtype) == 2
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
    handle.overwriteArrowTable(new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
    }))

    // One Arrow JS batch per stream; intoTable is the caller's choice.
    assert.equal([...handle.readArrowReader()].length, 1)
    assert.equal(handle.readArrowReader().intoTable().numRows, 2)
    assert.equal(handle.readArrowField().dtype.length, 2)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Edges

- `read_records` -> Python and JavaScript only; the Rust primitive read surface stays Arrow-native.
- declared `options.field` -> `read_arrow_field` returns it without reading the file, so an empty handle answers without a footer.
- read `field` naming a column the file does not store -> no mask, so every chunk is read and the saving is silently lost; the result still takes the declared shape, and the unstored column comes back all-null ([Pushdown](pushdown.md)).
- absent resource -> zero batches under the declared schema, and no missing-footer error ([Footer](footer.md#absent-and-empty-files)).
- `read_scalar` on a `.parquet` name -> refused; a file is records, not one structured document.
- a coded name such as `trades.parquet.gz` -> refused before a byte is decoded ([Compression](compression.md)).
- `row_size` and `column_size` -> footer answers about the whole file; a projection, a filter, or a row limit does not move them ([Parquet](index.md)).

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test parquet -- mod_::internal
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_dimensions/parquet/read_rows
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_init.py python/tests/test_iomedia.py
    python/.venv/bin/python python/benchmarks/media.py --filter "parquet read whole" --filter "parquet read records"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/records.test.js
    YGGDRYL_BENCH_FILTER=records/read_parquet_into_ipc npm run --prefix node bench:media
    ```
