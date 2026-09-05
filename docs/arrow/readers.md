# Readers

`BatchReader` is the one shape of a record read or write: a schema plus an iterator of `Result<RecordBatch>`.

## Contract

| Key | Value |
| --- | --- |
| Owns | `BatchReader`, `batch_reader`, `combined`, `combined_as`, `cast_reader` |
| Shape | `Box<dyn arrow_array::RecordBatchReader + Send>`; owns its source, outlives the call, `Send` |
| Returned by | `ipc::read_batch_reader`, `parquet::read_batch_reader` (`parquet` feature), `IOMedia::read_arrow_reader` |
| Consumed by | `overwrite_arrow_reader`, `append_arrow_reader`, `merge_arrow_reader` ([../holder/iobase/records.md](../holder/iobase/records.md)) |
| Lazy | Schema before any batch; `combined` merges without pulling a row; nothing is collected |
| Cast | `cast_reader(inner, &field, safe)` ([../types/cast.md](../types/cast.md)); an exact side passes through unchanged |
| Bindings | Rust; Python `combined(left, right, schema=None, *, safe=True)`; JavaScript `BatchReader.combined(other, schema?, safe?)` |

## Use

`combined` chains two readers onto the root their schemas merge into; `combined_as` chains onto a root the caller holds.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::DataType;

    let left_root = DataType::from_fields([DataType::Int64.nullable_field("id")])?
        .required_field("row");
    let right_root = DataType::from_fields([
        DataType::Int64.nullable_field("id"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");

    let left_schema = left_root.into_arrow_schema()?;
    let right_schema = right_root.into_arrow_schema()?;
    let left = yggdryl::arrow::batch_reader(
        Arc::clone(&left_schema),
        [RecordBatch::try_new(left_schema, vec![Arc::new(Int64Array::from(vec![1_i64]))])?],
    );
    let right = yggdryl::arrow::batch_reader(
        Arc::clone(&right_schema),
        [RecordBatch::try_new(
            right_schema,
            vec![
                Arc::new(Int64Array::from(vec![2_i64])),
                Arc::new(StringArray::from(vec!["XPAR"])),
            ],
        )?],
    );

    let joined = yggdryl::arrow::combined(left, right)?;
    assert_eq!(joined.schema().fields().len(), 2);
    // The left's rows carry no `venue`, so they read null for it.
    let batches: Vec<_> = joined.collect::<Result<_, _>>()?;
    assert!(batches[0].column(1).is_null(0));
    assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import combined

    left = pa.table({"id": [1]})
    right = pa.table({"id": [2], "venue": ["XPAR"]})

    joined = combined(left, right).read_all()
    assert joined.column_names == ["id", "venue"]
    assert joined.num_rows == 2
    # The left's row has no `venue`, so it reads null.
    assert joined.column("venue").to_pylist() == [None, "XPAR"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { BatchReader, DataType, Field } = require('yggdryl')
    const { tableFromArrays, tableToIPC } = require('apache-arrow')

    const left = BatchReader.from(tableFromArrays({ id: BigInt64Array.from([1n]) }))
    const right = BatchReader.from(tableFromArrays({ id: BigInt64Array.from([2n]) }))

    const joined = left.combined(right)
    assert.equal(joined.field.dtype.length, 1)
    assert.equal(joined.intoTable().numRows, 2)
    ```

## Merge rules

| Rule | Behavior |
| --- | --- |
| Column identity | Name, ASCII case-insensitive; left's order, then right-only columns in right's order |
| Shared column | One datatype; a difference is refused naming both sides, never widened |
| One-sided column | Nullable, even when non-nullable on its side; the other side reads null |
| Metadata, field ids | Left's; a conflicting `PARQUET:field_id` is refused, never reassigned |
| Root | Left's name; bounded non-nullable Struct; appendable to Iceberg wherever both inputs were |

## Streaming batches

`arrow::batch_reader(schema, batches)` takes any `IntoIterator` of owned batches, so a generator encodes as it produces. Rust only.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::media::ipc::{self, IpcOptions};
use yggdryl::{DataType, Field};

let schema = Field::new(
    "row",
    DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?,
    false,
);
let projected = schema.into_arrow_schema()?;

let batch = |ids: Vec<i64>, symbols: Vec<Option<&str>>| {
    RecordBatch::try_new(
        Arc::clone(&projected),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(symbols)),
        ],
    )
};

let mut handle = Buffer::new();
let options = IpcOptions::new();
ipc::overwrite_arrow_reader(
    &mut handle,
    yggdryl::arrow::batch_reader(
        Arc::clone(&projected),
        [batch(vec![1, 2], vec![Some("AAPL"), None])?, batch(vec![3], vec![None])?],
    ),
    &options,
)?;

// A BatchReader knows its schema before it yields anything.
let reader = ipc::read_batch_reader(&handle, None, &options)?;
assert_eq!(reader.schema().as_ref(), projected.as_ref());

let mut rows = 0;
for batch in reader {
    rows += batch?.num_rows();
}
assert_eq!(rows, 3);
```

## Edges

- Root not a bounded, non-nullable Struct -> `combined_as` and `cast_reader` return `Err`.
- `parquet::read_batch_reader` -> absent unless the non-default `parquet` feature is on.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test arrow combined::
    cargo test --features "parquet iceberg" -p yggdryl --lib arrow::rows::
    cargo test --features "parquet iceberg" -p yggdryl --test arrow cast_coverage::
    ```
