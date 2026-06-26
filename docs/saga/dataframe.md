# DataFrame (eager)

`DataFrame` is the first concrete [`Frame`](frame.md) backing: an **eager**,
in-memory table over an Arrow [`RecordBatch`]. It is cheap to clone (columns are
reference-counted), projection and row-slicing are **zero-copy** (they reuse the
Arrow buffers), and `filter` types the predicate against the schema before applying
it. Each column it yields is an [`ArrayColumn`](column.md) — a materialized
[`Column`](column.md) over an Arrow array.

Gated behind the on-by-default `dataframe` feature (it pulls the Arrow columnar
kernels). `default-features = false` leaves the schema vocabulary and the
`Frame`/`Column` traits without a concrete backing.

!!! note
    Python and Node bindings for `yggdryl-saga` are planned; the examples below are
    Rust, the source of truth.

## Build

From a [`Schema`](schema.md) and one Arrow array per field, or by wrapping an
existing `RecordBatch`:

=== "Rust"

    ```rust
    use std::sync::Arc;
    use arrow_array::{Int64Array, StringArray};
    use yggdryl_saga::{DataFrame, Frame, Schema};

    let df = DataFrame::new(
        Schema::from_str("id: int64 not null, symbol: utf8 not null, px: int64").unwrap(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
            Arc::new(Int64Array::from(vec![100, 200, 300])),
        ],
    )
    .unwrap();

    assert_eq!(df.height(), Some(3));
    assert_eq!(df.width().unwrap(), 3);
    ```

## Project, slice, access

`select` / `drop` project columns; `head` / `tail` / `slice` take row ranges
(zero-copy); `column(name)` hands back a materialized `ArrayColumn`.

=== "Rust"

    ```rust
    # use std::sync::Arc;
    # use arrow_array::{Int64Array, StringArray};
    # use yggdryl_saga::{Column, DataFrame, Frame, Schema};
    # let df = DataFrame::new(
    #     Schema::from_str("id: int64 not null, symbol: utf8 not null, px: int64").unwrap(),
    #     vec![Arc::new(Int64Array::from(vec![1,2,3])), Arc::new(StringArray::from(vec!["AAPL","MSFT","AAPL"])), Arc::new(Int64Array::from(vec![100,200,300]))],
    # ).unwrap();
    let top = df.clone().select(&["symbol", "px"]).unwrap().head(2).unwrap();
    assert_eq!(top.width().unwrap(), 2);

    let px = df.column("px").unwrap();
    assert_eq!(px.len(), Some(3));
    ```

## Filter

`filter` takes a [`Predicate`](predicate.md) and does the whole job: it
type-optimises the literals against the schema (so an untyped ISO string becomes a
typed `timestamp`, a string `"150"` an `int64`, …) and then evaluates the predicate
into a keep-mask. Comparisons, `BETWEEN`, `IN`/`NOT IN`, `IS NULL` and `and`/`or`/`not`
all work; a null cell never matches a comparison.

=== "Rust"

    ```rust
    # use std::sync::Arc;
    # use arrow_array::{Int64Array, StringArray};
    # use yggdryl_saga::{DataFrame, Frame, Predicate, Scalar, Schema};
    # let df = DataFrame::new(
    #     Schema::from_str("symbol: utf8 not null, px: int64").unwrap(),
    #     vec![Arc::new(StringArray::from(vec!["AAPL","MSFT","AAPL"])), Arc::new(Int64Array::from(vec![100,200,300]))],
    # ).unwrap();
    let out = df
        .filter(
            Predicate::between("px", Scalar::any("100"), Scalar::any("250"))
                .and(Predicate::eq("symbol", Scalar::utf8("MSFT"))),
        )
        .unwrap();
    assert_eq!(out.height(), Some(1)); // MSFT @ 200
    ```

!!! note "First cut"
    The filter evaluator is **row-wise** — correct over the common flat column types
    (integers, floats, booleans, strings, dates/timestamps, `decimal128`).
    Vectorising it over Arrow compute kernels, and pushing predicates into a lazy
    plan / file source, are later steps.

## Aggregate

`group_by(keys)` and `resample(time, every)` reduce rows into groups, finished with
`.agg([...])` — see [Group-by & resample](aggregate.md). Both take a single-pass,
hash-free path over sorted timeseries.

## Arrow interop

`record_batch()` / `into_record_batch()` expose the underlying Arrow data, and
`from_record_batch` wraps one back up — all without copying.

## Next

- [Group-by & resample](aggregate.md) — `group_by` / `resample` aggregation

- [Frame](frame.md) — the trait `DataFrame` implements
- [Predicate](predicate.md) — the filters it consumes (and types)
- [Column](column.md) — the `ArrayColumn` it yields
