# Writing Arrow IPC

Rows into a stream - as native scalars, as Arrow batches - with the intent in the method name.

## Contract

| Key | Value |
| --- | --- |
| Owns | `ipc::overwrite_arrow_reader`, the one complete-stream encoder every intent publishes through |
| Native rows | `overwrite_records`, `append_records`, `merge_records`, `write_records`; a row is anything that converts into a [`Scalar`](../../types/scalar.md) |
| Batches | `overwrite_arrow_reader`, `append_arrow_reader`, keyed `merge_arrow_reader` from [`IOMedia`](../../holder/iobase/records.md) |
| Intent | the method name carries it; `merge_by` supplies identity only, never the choice |
| Row shape | an ordered `Scalar::Sequence` under the root, or a name-sorted `Scalar::Struct` resolved to that order |
| Schema | `options.field` declares the root; a non-empty mapping or dataclass source infers it, a positional one cannot |
| Lazy | a batch is encoded as the writer pulls it; only the current one is alive |
| Batching | rows are grouped into batches bounded by `batch_row_size` and published on the `commit_row_size` cadence |
| Coding | applied from the name on write, with `level` as the only compression setting ([Options](options.md#content-coding-comes-from-the-name)) |
| Documents | `write_scalar` is a structured-text call; an IPC name refuses it |

## Use

Native rows first - a tuple, a mapping, a dataclass, a plain object - with no Arrow type in the call. Append retains the stored rows; keyed merge updates matching `id` values and inserts misses.

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

    // Rust reads Arrow, then crosses into the value model in one call: a stream
    // answers a sequence of ordered row sequences.
    let rows = handle.read_arrow(Some(&options.clone().with_field(field.clone())))?.into_scalar()?;
    let venues = rows
        .as_sequence()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row.as_sequence().and_then(|row| row[1].as_str()))
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 3);
    assert!(venues.contains(&"XPAR") && !venues.contains(&"XNYS"));
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

## Rows as Arrow batches

The three intents share one reader-shaped input. Nothing is materialized: each batch is encoded as the writer pulls it, and `append_*` and keyed `merge_*` both publish through `ipc::overwrite_arrow_reader`, the one complete-stream encoder.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
    .required_field("row");
    let schema = field.into_arrow_schema()?;
    let batch = |ids: Vec<i64>, venues: Vec<Option<&str>>| {
        RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(venues)),
            ],
        )
    };

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![1, 2], vec![Some("XNAS"), Some("XNYS")])?],
        ),
        &options,
    )?;
    handle.append_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![3], vec![Some("XLON")])?],
        ),
        &options,
    )?;
    handle.merge_arrow_reader(
        yggdryl::arrow::batch_reader(
            Arc::clone(&schema),
            [batch(vec![2, 4], vec![Some("XPAR"), None])?],
        ),
        &options.clone().with_merge_by(["id"])?,
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

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("venue", pa.string()),
    ])
    batch = lambda ids, venues: pa.record_batch(
        {"id": ids, "venue": venues}, schema=schema
    )

    # The name says Arrow IPC, so no call names an encoding.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(batch([1, 2], ["XNAS", "XNYS"]))
    handle.append_arrow_batch(batch([3], ["XLON"]))

    merging = handle.record_options()
    merging.merge_by = ["id"]
    handle.merge_arrow_batch(batch([2, 4], ["XPAR", None]), options=merging)

    assert handle.read_arrow_field().name == "row"
    assert handle.read_arrow_reader().read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const rows = (ids, venues) => new arrow.Table({
      id: arrow.vectorFromArray(ids.map(BigInt), new arrow.Int64()),
      venue: arrow.vectorFromArray(venues, new arrow.Utf8()),
    })

    // The name says Arrow IPC, so no call names an encoding.
    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const handle = new IOBase(path.join(root, 'trades.arrows'))
    handle.overwriteArrowTable(rows([1, 2], ['XNAS', 'XNYS']))
    handle.appendArrowTable(rows([3], ['XLON']))
    handle.mergeArrowTable(
      rows([2, 4], ['XPAR', null]),
      handle.recordOptions().withMergeBy(['id']),
    )

    assert.equal(handle.readArrowField().name, 'row')
    assert.equal(handle.readArrowReader().intoTable().numRows, 4)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Shaping and cadence

Row conversion is bounded by the smaller of `batch_row_size` and `commit_row_size`, so a failed row keeps the committed prefix. Both settings, and every other one, live on the shared [`RecordOptions`](../options.md); [Records](../../holder/iobase/records.md#commit-cadence) states the publication boundary once for every encoding.

## Edges

- empty row source without `options.field` -> refused; a non-empty mapping or dataclass source infers it.
- positional rows without `options.field` -> refused naming the missing columns; names have to come from somewhere.
- `merge_by` on an overwrite or append -> refused; the key never selects merge, and intent stays with the method name.
- `append_*` or keyed `merge_*` -> published through `ipc::overwrite_arrow_reader`, the one complete-stream encoder.
- `write_scalar` on an `.arrows` name -> refused; a stream is records, not one structured document.
- zero batches written -> the schema is still written, so the stream exists and reads; [Absence](options.md#absence) shows it.
- a handle whose name declares no coding -> `level` does nothing ([Options](options.md#content-coding-comes-from-the-name)).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib ipc::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_record
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_records
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_stateful/ipc
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_io_records.py
    python/.venv/bin/python -m pytest python/tests/media/test_ipc.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    ```
