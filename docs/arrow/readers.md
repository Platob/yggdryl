# Readers

`BatchReader` is the one shape of a record read or write: a schema plus an iterator of `Result<RecordBatch>`.

## Contract

| Key | Value |
| --- | --- |
| Owns | `BatchReader`, `batch_reader`, `combined`, `combined_as` |
| Shape | `Box<dyn arrow_array::RecordBatchReader + Send>`; owns what it reads from |
| Paths | Every read path returns one; `overwrite_arrow_reader`, `append_arrow_reader`, `merge_arrow_reader` consume one ([../holder/index.md#records](../holder/index.md#records)) |
| Feature flag | `parquet::read_batch_reader` needs the non-default `parquet` feature |
| Roots | `combined` merges both schemas into the root; `combined_as` casts both onto the caller's |
| Lazy | Schema before any batch; `combined` pulls no row and collects nothing |
| Cast | [`SerieReader`](../types/cast.md#eager-and-lazy)`::from_arrow_reader(root, inner, options)`: one compiled plan for the whole stream, one record `Serie` per batch; `into_arrow_reader` hands it back as a `BatchReader`, and an identity plan hands back the inner reader unwrapped |
| Cast errors | Reported at the pull of the batch that carries them; the reader is fused after one, and the source is released then |
| Bindings | Rust; Python `combined(left, right, schema=None, *, safe=True)` and `SerieReader.from_arrow_reader(reader, root=None, ...)`; JavaScript `BatchReader.combined(other, schema?, safe?)` and `SerieReader.fromArrowReader(reader, root?, options?)` |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::{DataType, StructType};

    let left_root = DataType::from(StructType::from_fields([DataType::Int64.nullable_field("id")])?)
        .required_field("row");
    let right_root = DataType::from(StructType::from_fields([
        DataType::Int64.nullable_field("id"),
        DataType::utf8().nullable_field("venue"),
    ])?)
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
    const { BatchReader } = require('yggdryl')
    const { tableFromArrays } = require('apache-arrow')

    const left = BatchReader.from(tableFromArrays({ id: BigInt64Array.from([1n]) }))
    const right = BatchReader.from(tableFromArrays({ id: BigInt64Array.from([2n]) }))

    const joined = left.combined(right)
    assert.equal(joined.field.dtype.length, 1)
    assert.equal(joined.intoTable().numRows, 2)
    ```

## Casting a stream

`SerieReader` is a stream reconciled to a non-null Struct root: the plan is compiled from the
reader's schema before a batch is pulled, so a cast the two schemas alone refuse is refused by the
constructor, and each pulled batch is one record [`Serie`](../types/serie.md). With no root it is
the stream's own schema, read as the record `row`. `into_arrow_reader` is the transport face - the
same batches reconciled to the root as they are pulled and never landed - which is what a record
write takes. [Eager and lazy](../types/cast.md#eager-and-lazy) has the failure timing.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Int32Array, RecordBatch, RecordBatchReader};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::arrow::batch_reader;
    use yggdryl::{ArrowCastOptions, DataType, SerieReader, StructType};

    let root = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let schema = Arc::new(Schema::new(vec![ArrowField::new(
        "id",
        ArrowDataType::Int32,
        false,
    )]));
    let first = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int32Array::from(vec![1, 2])) as ArrayRef],
    )?;
    let second = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int32Array::from(vec![3])) as ArrayRef],
    )?;

    // One record column per batch, every one cast by the one plan.
    let series = SerieReader::from_arrow_reader(
        Some(&root),
        batch_reader(Arc::clone(&schema), [first.clone(), second.clone()]),
        ArrowCastOptions::new(),
    )?;
    assert_eq!(series.field(), &root);
    let mut lengths = Vec::new();
    for serie in series {
        lengths.push(serie?.len());
    }
    assert_eq!(lengths, [2, 1]);

    // The transport face states the root's schema before a batch is pulled.
    let reader = SerieReader::from_arrow_reader(
        Some(&root),
        batch_reader(schema, [first, second]),
        ArrowCastOptions::new(),
    )?
    .into_arrow_reader();
    assert_eq!(reader.schema().field(0).data_type(), &ArrowDataType::Int64);
    let mut rows = 0;
    for batch in reader {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 3);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, SerieReader

    root = Field("row", "struct<id: int64>", nullable=False)
    table = pa.table({"id": pa.array([1, 2, 3], pa.int32())})

    # One record column per batch, every one cast by the one plan.
    series = SerieReader.from_arrow_reader(table.to_reader(max_chunksize=2), root)
    assert series.field == root
    assert [serie.child("id").as_py() for serie in series] == [[1, 2], [3]]

    # The transport face is a pyarrow reader that casts as it is read.
    reader = SerieReader.from_arrow_reader(table, root).into_arrow_reader()
    assert isinstance(reader, pa.RecordBatchReader)
    assert reader.schema.field("id").type == pa.int64()
    assert reader.read_all().num_rows == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, SerieReader, fields } = require('yggdryl')

    const root = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const chunk = (ids) => new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int32()) })
    const source = () => new arrow.Table([...chunk([1, 2]).batches, ...chunk([3]).batches])

    // One record column per batch, every one cast by the one plan.
    const series = SerieReader.fromArrowReader(BatchReader.from(source()), root)
    assert.ok(series.field.equals(root))
    assert.deepEqual(
      [...series].map((serie) => serie.child('id').asJs()),
      [[1, 2], [3]],
    )

    // The transport face is a native BatchReader, read once.
    const reader = SerieReader.fromArrowReader(BatchReader.from(source()), root).intoArrowReader()
    assert.ok(reader instanceof BatchReader)
    assert.equal(reader.intoTable().numRows, 3)
    ```

## Merge rules

| Rule | Behavior |
| --- | --- |
| Column identity | Name, ASCII case-insensitive; left's order, then right-only columns in right's order |
| Shared column | One datatype, never widened |
| One-sided column | Nullable, even when non-nullable on its side; the other side reads null |
| Metadata, field ids | Left's kept |
| Root | Left's name; bounded non-nullable Struct; appendable to Iceberg wherever both inputs were |

## Streaming batches

`arrow::batch_reader(schema, batches)` takes any `IntoIterator` of owned batches, so a generator encodes as it produces. Rust only.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
use yggdryl::holder::Buffer;
use yggdryl::StructType;
use yggdryl::ipc::{self, IpcOptions};
use yggdryl::DataType;

let projected = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row")
    .into_arrow_schema()?;
let batch = |ids: Vec<i64>| {
    RecordBatch::try_new(Arc::clone(&projected), vec![Arc::new(Int64Array::from(ids))])
};

// Two batches, written as the iterator yields them.
let mut handle = Buffer::new();
let options = IpcOptions::new();
ipc::overwrite_arrow_reader(
    &mut handle,
    yggdryl::arrow::batch_reader(Arc::clone(&projected), [batch(vec![1, 2])?, batch(vec![3])?]),
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

- Shared column with two datatypes or two `PARQUET:field_id` values -> `combined` refuses, naming both sides.
- Root not a bounded, non-nullable Struct -> `combined_as` and `SerieReader::from_arrow_reader` return `Err`.
- A cast the two schemas alone refuse - an unsupported conversion, an ambiguous name, a required column missing under [`strict`](../types/cast.md#strict-nullability) -> `SerieReader::from_arrow_reader` returns `Err` rather than a reader that fails on its first batch.
- A batch the plan refuses -> reported at the pull that reads it, and the reader is fused after it.
- Dropping a `SerieReader` or its transport face before it is drained -> the source is dropped with it, so a C stream behind it is released there.
- Python batch export caches the exact schema before any pull, retains [nested Map flags and shared buffers](../types/serie.md#exact-map-schemas), and releases the native reader on exhaustion or failure. A batch with no columns still retains its row count.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test arrow -- mod_
    cargo test --features "iceberg internals parquet" -p yggdryl --test arrow -- rows::row_values rows::widening
    cargo test --features "parquet iceberg" -p yggdryl --test root -- cast::coverage
    cargo test --features "parquet iceberg" -p yggdryl --test root -- cast::plans
    cargo test --features "parquet iceberg" -p yggdryl --test serie -- arrow::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_cast.py -k reader
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/serie.test.js
    ```
