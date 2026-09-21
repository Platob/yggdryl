# Reading Arrow IPC

Rows out of a stream - as native scalars, as Arrow batches - and the schema on its own.

## Contract

| Key | Value |
| --- | --- |
| Owns | `ipc::read_field`, `ipc::read_batch_reader`; `read_records`, `read_arrow_reader`, `read_arrow_field` from [`IOMedia`](../../holder/iobase/records.md) |
| Native rows | Python `read_records`, JavaScript `readRecords`; Rust reads Arrow and crosses with `ArrowScalar::into_scalar` |
| Row shape | a mapping per row in the bindings; an ordered `Scalar::Sequence` under the root in the [value model](../../types/scalar.md) |
| Batches | an [`arrow::BatchReader`](../../arrow/readers.md) whose schema is known before the first batch |
| Lazy | a batch is decoded as the reader is stepped; only the current one is alive |
| Schema | recovered from the bytes; `options.field` declared instead answers without touching the handle |
| Projection | `options.field` naming a subset is a [column pushdown](pushdown.md), never a cast |
| Absence | a location that holds nothing yields no rows; [Absence](options.md#absence) states it once |
| Bindings | Python `pyarrow.RecordBatchReader` over the Arrow C Stream; JavaScript Arrow JS over the copied IPC bytes |

## Use

Native rows first: Python and JavaScript hand back one mapping per row, and Rust reads Arrow and crosses into the value model in the same call. Nothing in the call names an encoding - the name already did.

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

    // Rust reads Arrow, then crosses into the value model in one call: a stream
    // answers a sequence of ordered row sequences.
    let rows = handle.read_arrow(Some(&options))?.into_scalar()?;
    let venues = rows
        .as_sequence()
        .unwrap_or_default()
        .iter()
        .filter_map(|row| row.as_sequence().and_then(|row| row[1].as_str()))
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 2);
    assert_eq!(venues, ["XNAS", "XNYS"]);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_records([{"id": 1, "venue": "XNAS"}, {"id": 2, "venue": "XNYS"}])

    # Plain dictionaries back, one row at a time.
    rows = list(handle.read_records())
    assert len(rows) == 2
    assert [row["venue"] for row in rows] == ["XNAS", "XNYS"]
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
    handle.overwriteRecords([{ id: 1n, venue: 'XNAS' }, { id: 2n, venue: 'XNYS' }])

    // Plain objects back, one row at a time.
    const rows = [...handle.readRecords()]
    assert.equal(rows.length, 2)
    assert.deepEqual(rows.map((row) => row.venue), ['XNAS', 'XNYS'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Rows as Arrow batches

The same rows without the crossing: a read returns a reader, and stepping it decodes one batch at a time, block boundaries included. Nothing is collected on either side.

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

## The schema alone

`read_arrow_field` reads the leading messages and stops, so asking what a stream holds never decodes a row. `ipc::read_field` is the same answer without the `IOMedia` layer above it.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::holder::Buffer;
    use yggdryl::ipc::{self, IpcOptions};
    use yggdryl::{DataType, IOBase, IOMedia, MimeType, StructType};

    let field = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let schema = field.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int64Array::from(vec![1, 2]))],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let options = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(schema, [batch]), &options)?;

    // The schema, and not one row.
    assert_eq!(handle.read_arrow_field(&options)?, field);

    // The encoding seam answers the same field.
    let bare = ipc::read_field(&handle, &IpcOptions::new())?;
    assert_eq!(bare.name(), "row");
    assert_eq!(bare.field_len(), 1);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_table(pa.table({"id": [1, 2]}))

    # The schema, and not one row.
    field = handle.read_arrow_field()
    assert field.name == "row"
    assert [child.name for child in field.dtype] == ["id"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { IOBase, MimeType } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowTable(
      new arrow.Table({ id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()) }),
    )

    // The schema, and not one row.
    const field = handle.readArrowField()
    assert.equal(field.name, 'row')
    assert.deepEqual([...field.dtype].map((child) => child.name), ['id'])
    ```

## Edges

- `read_records` -> Python and JavaScript only; the Rust primitive read surface stays Arrow-native and crosses with `read_arrow(...).into_scalar()`.
- `IpcOptions::field` declared -> `read_field` answers it without touching the handle.
- reading -> batches arrive one at a time and only the current one is alive.
- `read_scalar` on an `.arrows` name -> refused; a stream is records, not one structured document.
- absent resource -> no rows, not an error; [Absence](options.md#absence) shows the whole shape.
- bytes that are not a stream -> `read_field` and `read_batch_reader` error at once; [Absence](options.md#absence) shows each binding's failure.
- a projected read -> its own narrower schema, on [Pushdown](pushdown.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib ipc::tests
    cargo bench --features "parquet iceberg" -p yggdryl --bench media -- io_pushdown/ipc
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/media/test_ipc.py
    python/.venv/bin/python -m pytest python/tests/media/test_io_records.py
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/media/records.test.js
    ```
