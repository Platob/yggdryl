# Apache Parquet writes

Rows into a Parquet file: native scalars first, then Arrow batches, under the three intents every encoding shares.

## Contract

| Item | Behaviour |
| --- | --- |
| Native rows | `overwrite_records`, `append_records`, `merge_records`, `write_records`; a row is anything that converts into a [`Scalar`](../../types/scalar.md) |
| Batches | `overwrite_arrow_reader`, `append_arrow_reader`, keyed `merge_arrow_reader` under the [canonical signatures](../../holder/iobase/records.md) |
| Intents | the three [every encoding has](../../holder/iobase/records.md#write-intents): append rewrites the file around the stored rows, keyed merge upserts by `merge_by` |
| Schema | `options.field` declares the root; a non-empty mapping or dataclass source infers it, a positional one cannot |
| Validated | a batch naming the same columns as the written root is cast strictly; one naming different columns is refused, with the batch index in the message |
| Row groups | `max_row_group_size`, default 1,048,576, bounds what one group holds; `batch_row_size` bounds what one batch carries |
| Cadence | row conversion is bounded by the smaller of `batch_row_size` and `commit_row_size`, and `commit_row_size` decides what a failure leaves published |
| Result | a whole file - magic, pages, footer - so any Parquet reader opens it |
| Field ids | a root carrying `PARQUET:field_id` writes those ids into the file schema: [Footer](footer.md#field-identifiers) |
| Compression | pages are compressed inside the file through `compression`; an outer coding on the handle is refused: [Compression](compression.md) |
| Refused | `merge_by` on an overwrite or an append; `write_scalar` on a Parquet name, which is a structured-text call |

## Use

Native rows go in one value at a time, and the three intents take them the same way.

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
    handle.append_records([Trade(3, "GOOG")], &options)?;
    handle.merge_records([Trade(2, "NVDA")], &options.clone().with_merge_by(["id"])?)?;

    // Rust reads Arrow, then crosses into the value model in one call: a file
    // answers a sequence of ordered row sequences.
    let rows = handle.read_arrow(Some(&options.clone().with_field(field.clone())))?.into_scalar()?;
    let symbols = rows
        .as_sequence()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row.as_sequence().and_then(|row| row[1].as_str()))
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 3);
    assert!(symbols.contains(&"NVDA") && !symbols.contains(&"MSFT"));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.parquet")

    # Mappings are rows, and a non-empty source infers the field from them.
    handle.overwrite_records([{"id": 1, "symbol": "AAPL"}, {"id": 2, "symbol": "MSFT"}])
    handle.append_records([{"id": 3, "symbol": "GOOG"}])

    merging = handle.record_options()
    merging.merge_by = ["id"]
    handle.merge_records([{"id": 2, "symbol": "NVDA"}], options=merging)

    # Plain dictionaries back, one row at a time.
    rows = list(handle.read_records())
    assert len(rows) == 3
    assert {row["symbol"] for row in rows} == {"AAPL", "NVDA", "GOOG"}
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

    // Plain objects are rows; the writers widen them into the file.
    handle.overwriteRecords([{ id: 1n, symbol: 'AAPL' }, { id: 2n, symbol: 'MSFT' }])
    handle.appendRecords([{ id: 3n, symbol: 'GOOG' }])
    handle.mergeRecords(
      [{ id: 2n, symbol: 'NVDA' }],
      handle.recordOptions().withMergeBy(['id']),
    )

    const rows = [...handle.readRecords()]
    assert.equal(rows.length, 3)
    assert.deepEqual(
      new Set(rows.map((row) => row.symbol)),
      new Set(['AAPL', 'NVDA', 'GOOG']),
    )

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Rows as Arrow batches

The same three intents take a batch reader instead of rows; what is Parquet's own is the result: a whole file any reader opens.

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

    assert_eq!(handle.read_arrow_reader(&options)?.count(), 1);
    assert_eq!(handle.read_arrow_field(&options)?.field_len(), 2);
    // A whole Parquet file, magic and footer included.
    assert_eq!(&handle.read_all_bytes()?[..4], b"PAR1");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa
    import pyarrow.parquet as pq

    from yggdryl import IOBase

    path = pathlib.Path(tempfile.mkdtemp()) / "trades.parquet"
    handle = IOBase(path)
    handle.overwrite_arrow_table(pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]}))
    handle.append_arrow_table(pa.table({"id": [3], "symbol": ["GOOG"]}))

    assert handle.read_arrow_reader().read_all().num_rows == 3
    # A whole Parquet file, so any other reader opens it.
    assert pq.read_table(path).num_rows == 3
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

    assert.equal(handle.readArrowReader().intoTable().numRows, 3)
    // A whole Parquet file, magic and footer included.
    assert.deepEqual([...handle.readBytes().subarray(0, 4)], [...Buffer.from('PAR1')])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Shaping and cadence

Row conversion is bounded by the smaller of `batch_row_size` and `commit_row_size`, so a failed row keeps the committed prefix. Both settings, and every other one, live on the shared [`RecordOptions`](../options.md); [Records](../../holder/iobase/records.md#commit-cadence) states the publication boundary once for every encoding.

Only a whole file is a Parquet file, so an append or a merge reads the stored rows and republishes them beside the new ones; [Arrow IPC](../ipc/write.md) appends a message instead. Inside the file, `max_row_group_size` decides how many row groups that republication produces, and [Footer](footer.md) reads them back.

## Edges

- empty row source without `options.field` -> refused; a non-empty mapping or dataclass source infers it.
- positional rows without `options.field` -> refused naming the missing columns.
- a pulled batch not matching the reader's field -> error naming the batch index; a batch naming the same columns is cast strictly instead.
- keyed `merge_arrow_reader` -> upsert: rows matching `merge_by` are updated, misses are inserted.
- `merge_by` on an overwrite or append -> refused; intent stays with the method name.
- write with no batches -> a real file whose footer holds the schema and `num_rows == 0` ([Footer](footer.md#absent-and-empty-files)).
- `write_scalar` on a `.parquet` name -> refused; a file is records, not one structured document.
- a coded name such as `trades.parquet.gz` -> refused before a row is pulled, and the handle is left untouched ([Compression](compression.md)).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib parquet::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_records
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/parquet
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_io_records.py python/tests/media/test_parquet.py
    python/.venv/bin/python python/benchmarks/media.py --filter "parquet write" --filter "PyArrow parquet"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    ```
