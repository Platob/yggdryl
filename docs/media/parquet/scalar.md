# Apache Parquet scalars

Native rows - a tuple, a mapping, a dataclass, a plain object - in and out of a Parquet file, with no Arrow type in the call.

## Contract

| Key | Value |
| --- | --- |
| Writes | `overwrite_records`, `append_records`, `merge_records`, `write_records`; a row is anything that converts into a [`Scalar`](../../types/scalar.md) |
| Reads | Python `read_records`, JavaScript `readRecords`; Rust reads Arrow and crosses with `ArrowScalar::into_scalar` |
| Row shape | an ordered `Scalar::Sequence` under the root, or a name-sorted `Scalar::Record` resolved to that order |
| Schema | `options.field` declares the root; a non-empty mapping or dataclass source infers it, a positional one cannot |
| Batching | rows are grouped into batches bounded by `batch_row_size`, then into row groups bounded by `max_row_group_size` |
| Documents | `read_scalar` and `write_scalar` are structured-text calls; a Parquet name refuses them |

## Use

The three intents carry the same rows as the [batch surface](arrow.md), one row value at a time.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{DataType, IOBase, IOMedia, Scalar, Url};

    struct Trade(i64, &'static str);

    impl From<Trade> for Scalar {
        fn from(row: Trade) -> Self {
            Scalar::from_sequence([Scalar::from(row.0), Scalar::from(row.1)])
        }
    }

    let field = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
    ])?
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

## Shaping and cadence

Row conversion is bounded by the smaller of `batch_row_size` and `commit_row_size`, so a failed row keeps the committed prefix. Both settings, and every other one, live on the shared [`RecordOptions`](../options.md); [Records](../../holder/iobase/records.md#commit-cadence) states the publication boundary once for every encoding.

Only a whole file is a Parquet file, so an append or a merge reads the stored rows and republishes them beside the new ones; [Arrow IPC](../ipc/scalar.md) appends a message instead.

## Edges

- empty row source without `options.field` -> refused; a non-empty mapping or dataclass source infers it.
- positional rows without `options.field` -> refused naming the missing columns.
- `read_records` -> Python and JavaScript only; the Rust primitive read surface stays Arrow-native.
- `read_scalar` on a `.parquet` name -> refused; a file is records, not one structured document.
- a coded name such as `trades.parquet.gz` -> refused before a row is pulled; Parquet compresses its own pages.
- absent resource -> no rows, not an error.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib parquet::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_write_records
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_io_records.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    ```
