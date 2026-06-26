# Group-by & resample

A [`DataFrame`](dataframe.md) reduces rows into groups two ways — both finished
with `.agg([...])`:

- **`group_by(keys)`** — one output row per distinct key combination; the key
  columns are carried through with their original types.
- **`resample(time, every)`** — timeseries bucketing: one output row per fixed
  `every`-wide window of a sorted `timestamp` / `date` column, the bucket start
  carried as the time column.

The aggregations are [`Agg`](#aggregations) values; the bucket width is a
[`Period`](#period) (`1h`, `5m`, `100ms`, …).

!!! note
    Python and Node bindings for `yggdryl-saga` are planned; the examples below are
    Rust, the source of truth.

## Aggregations

`Agg::count()` / `sum` / `min` / `max` / `mean` (over a column), with an optional
`.alias(name)`. `count` is `int64`; the rest are `float64` (computed in floating
point, nulls skipped — a group with no value is null). The output column is named
`<column>_<func>` (or `count`) unless aliased.

=== "Rust"

    ```rust
    use yggdryl_saga::Agg;

    let _ = [Agg::count(), Agg::sum("qty"), Agg::mean("px").alias("avg_px")];
    ```

## group_by

=== "Rust"

    ```rust
    # use std::sync::Arc;
    # use arrow_array::{Float64Array, Int64Array, StringArray};
    # use yggdryl_saga::{Agg, DataFrame, Frame, FrameHandle, Schema};
    let df = DataFrame::new(
        Schema::from_str("symbol: utf8 not null, px: float64, qty: int64").unwrap(),
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
            Arc::new(Float64Array::from(vec![10.0, 20.0, 12.0])),
            Arc::new(Int64Array::from(vec![1, 2, 3])),
        ],
    )
    .unwrap();

    let by_symbol = df
        .group_by(&["symbol"])
        .agg(&[Agg::count(), Agg::sum("qty"), Agg::mean("px").alias("avg_px")])
        .unwrap();

    assert_eq!(by_symbol.schema().unwrap().names(), ["symbol", "count", "qty_sum", "avg_px"]);
    assert_eq!(by_symbol.height(), Some(2)); // AAPL, MSFT
    ```

## resample

`resample` buckets a **sorted, non-null** time column. The bucket start is floored
to the period boundary and keeps the column's timestamp type, so the result is
itself a timeseries you can resample again, filter, or join.

=== "Rust"

    ```rust
    # use std::sync::Arc;
    # use arrow_array::{Float64Array, TimestampNanosecondArray};
    # use yggdryl_saga::{Agg, DataFrame, Frame, Period, Schema};
    # const H: i64 = 3_600 * 1_000_000_000;
    let df = DataFrame::new(
        Schema::from_str("ts: timestamp(ns, UTC) not null, px: float64").unwrap(),
        vec![
            Arc::new(TimestampNanosecondArray::from(vec![0, H, 25 * H]).with_timezone("UTC")),
            Arc::new(Float64Array::from(vec![10.0, 11.0, 20.0])),
        ],
    )
    .unwrap();

    // Daily OHLC-style reduction.
    let daily = df
        .resample("ts", Period::from_str("1d").unwrap())
        .agg(&[Agg::min("px").alias("low"), Agg::max("px").alias("high"), Agg::count()])
        .unwrap();
    assert_eq!(daily.height(), Some(2)); // two days
    ```

### Period

`Period::from_str` parses `<n><unit>` with unit `ns` / `us` / `ms` / `s` / `m` /
`h` / `d`. It is fixed-width (calendar-agnostic): `1d` is exactly 86 400 s. The
period must be a whole multiple of the time column's resolution.

## Timeseries optimisations

Both paths avoid hashing when the data is already ordered — the case sorted
timeseries are in:

- **`resample`** scans the sorted time column **once**: buckets are contiguous, so
  each window is a row *range*, reduced in a single linear pass (no hash map, no
  per-row index vectors). An out-of-order time column is rejected, so the fast path
  is always sound.
- **single-key `group_by`** detects a **sorted** key and takes the same contiguous
  run path; an unsorted key falls back to a first-seen-order hash group.

The reducers themselves are single-pass (`sum`/`count`/`min`/`max`/`mean` share one
scan per group).

## Next

- [DataFrame](dataframe.md) — the eager frame these run on
- [Frame](frame.md) — the trait the aggregated result still satisfies
