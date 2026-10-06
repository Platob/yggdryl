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
| Cast | [`StreamChunkedSerie`](../types/cast.md#eager-and-lazy)`::from_arrow_reader(root, inner, options)`: one compiled plan for the whole stream, one record `Serie` per batch; `into_arrow_reader` hands it back as a `BatchReader`, and an identity plan hands back the inner reader unwrapped |
| Cast errors | Reported at the pull of the batch that carries them; the reader is fused after one, and the source is released then |
| Windows | [`StreamChunkedSerie::window_by(by, sorted)`](#windows-of-a-stream): one lazy `StreamChunkedSerie` per window of equal adjacent keys, read in order, each stating its record as `static_values` |
| Bindings | Rust; Python `combined(left, right, schema=None, *, safe=True)` and `StreamChunkedSerie.from_arrow_reader(reader, root=None, ...)`; JavaScript `BatchReader.combined(other, schema?, safe?)` and `StreamChunkedSerie.fromArrowReader(reader, root?, options?)`; `window_by(by, sorted=False)` / `windowBy(by, sorted?)` and `static_values` / `staticValues` in both |

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

[StreamChunkedSerie](../types/stream-chunked-serie.md) is the native chunk stream.
`from_arrow_reader` compiles its cast before pulling a batch. The explicit
`into_arrow_reader` adapter hands the same source back to an Arrow consumer.

## Windows of a stream

`window_by` returns [StreamKeySerie](../types/key-serie.md), with a key record,
source paths, payload field and generic payload per adjacent window. Payloads are
read in order, without an Arrow batch per native row.

## Partitions of a stream

`partition_by` returns [StreamKeySerie](../types/key-serie.md). The same native
partitioner feeds partitioned folder and Iceberg writes. Bounds and clustering
close partitions without default row prebatching.

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
- Root not a bounded, non-nullable Struct -> `combined_as` and `StreamChunkedSerie::from_arrow_reader` return `Err`.
- A cast the two schemas alone refuse - an unsupported conversion, an ambiguous name, a [required column](../types/cast.md#required-columns) the source does not carry -> `StreamChunkedSerie::from_arrow_reader` returns `Err` rather than a reader that fails on its first batch.
- A batch the plan refuses -> reported at the pull that reads it, and the reader is fused after it.
- Dropping a `StreamChunkedSerie` or its transport face before it is drained -> the source is dropped with it, so a C stream behind it is released there.
- A window read after its walk passed rows of it -> refused once, `window 0 was passed by its walk with rows unread; read each window before taking the next`, then ended; read each window before taking the next.
- `window_by(.., sorted = true)` over keys arriving out of order -> the first window keyed backwards is refused naming the stream row, the batch, the row and both keys - `window by expects keys in order, ascending with absent keys last: batch 0 row 1 ...` - and the walk ends; window it unsorted, or hold it in a `ChunkedSerie` and window that sorted.
- `window_by`'s key is refused before any batch is pulled: text that is not a selector, a key stating no projection, an `unnest`, a term reaching no column, and a key cell folding onto `windownum`, `rownum` or a cell the windowed reader's record keeps, naming both - alias it. In Python, text that does not parse leaves the reader usable and any other refusal spends it, as a refused `cast` does.
- Python batch export caches the exact schema before any pull, retains [nested Map flags and shared buffers](../types/serie.md#exact-map-schemas), and releases the native reader on exhaustion or failure. A batch with no columns still retains its row count.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test arrow -- mod_
    cargo test --features "iceberg internals parquet" -p yggdryl --test arrow -- rows::row_values rows::widening
    cargo test --features "parquet iceberg" -p yggdryl --test root -- cast::coverage
    cargo test --features "parquet iceberg" -p yggdryl --test root -- cast::plans
    cargo test --features "parquet iceberg" -p yggdryl --test serie -- arrow::
    cargo test -p yggdryl --test allocations -- a_windowed_stream a_continuing_window_edge
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_cast.py -k reader
    python/.venv/bin/python -m pytest python/tests/test_serie.py -k TestReaderWindowBy
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/serie.test.js
    ```
