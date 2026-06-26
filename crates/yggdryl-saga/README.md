# yggdryl-saga

The columnar, **Arrow-convertible** dataframe core of the
[**yggdryl**](https://github.com/Platob/yggdryl) project: a lazy, zero-copy engine
aimed at sorted timeseries at large scale, with high-throughput streaming and the
analytics primitives a trading workload needs.

This first layer is the **schema vocabulary** — the value types that describe the
shape of a column, built to mirror Apache Arrow exactly so a schema crosses the
[`arrow-schema`](https://crates.io/crates/arrow-schema) boundary at zero cost.

It provides:

- `DataType` — the logical type of a column, split (as Arrow's own types are) into
  three families, one module each:
  - `PrimitiveType` — the flat, child-less scalars (`Int64`, `Float64`, `Utf8`,
    `FixedSizeBinary`, …);
  - `LogicalType` — semantic types over a physical layout (`Timestamp`, `Date32`,
    `Decimal128`, `Duration`, `Interval`, …);
  - `NestedType` — types carrying child `Field`s (`List`, `Struct`, `Map`,
    `Union`, `Dictionary`, `RunEndEncoded`, …);
- `Field` — a named, nullable `DataType` with metadata: the column header and the
  child element of every nested type;
- `Schema` — an ordered list of `Field`s with metadata (the arrow-`Schema`
  equivalent);
- `DataType::Any` — the **dynamic** type for an untyped literal, with
  `DataType::can_cast_to` the casting rule (numbers ↔ booleans ↔ strings, and
  strings/ints ↔ the temporal types — the ISO-date → `timestamp` path).

On top of that vocabulary sit the **base traits** every future frame and column
backing will satisfy, so eager and lazy implementations share one surface:

- `Column` — a single named, typed column, **materialized or lazy**: identity
  (`name` / `data_type` / `is_nullable`) is always known, `len()` is `Option`
  (unknown for an unevaluated lazy column), and `rename` / `cast` / `slice` /
  `head` / `tail` compose;
- `Frame` — a tabular frame: `select` / `drop` / `filter` / `limit` / column
  access over a common `Schema`, with structural defaults derived from the schema
  so an implementor supplies only the essentials. A generic `fn pipeline<F: Frame>`
  runs over any backing.

…and the **filtering layer** they consume:

- `Scalar` — a typed literal; `cast` types an untyped (`Any`) or string value
  (e.g. an ISO date → a `timestamp`);
- `Expression` / `col` / `lit` — expression nodes that resolve a type against a
  `Schema`;
- `Predicate` — a boolean filter whose `optimize(&schema)` casts each literal to
  its column's type, so `Frame::filter` can **push it down** into typed storage
  (`ParquetFrame`, `CsvFrame`).

The first concrete backing is built: the eager, Arrow-`RecordBatch`-backed
`DataFrame` / `ArrayColumn` (the on-by-default `dataframe` feature). Projection and
row-slicing are zero-copy, and `filter` types the predicate's literals against the
schema before evaluating it. A lazy frame and file sources (`ParquetFrame`,
`CsvFrame`) come next.

Every value type pairs a canonical-string `from_str` / `to_str` round-trip with, under
the on-by-default `arrow` feature, infallible `to_arrow()` / `from_arrow()`
conversions. Our `DataType` is a *total partition* of `arrow_schema::DataType`, so
the bridge is a lossless bijection in both directions.

```rust
use yggdryl_saga::{DataType, Field, PrimitiveType};

// Parse a nested schema from its canonical string …
let dt = DataType::from_str("list<item: struct<ts: timestamp(ns, UTC) not null, px: float64>>").unwrap();
assert!(dt.is_nested());

// … and hand it to Arrow at zero cost.
let arrow = dt.to_arrow();
assert_eq!(DataType::from_arrow(&arrow), dt);

let price = Field::new("price", DataType::from(PrimitiveType::Float64), false);
assert_eq!(price.to_str(), "price: float64 not null");
```

## Features

| feature | default | what it adds |
| --- | --- | --- |
| `arrow` | ✅ | zero-copy conversions to/from `arrow-schema`'s `DataType` / `Field` |
| `serde` | | `Serialize` / `Deserialize` for every schema type (structural, lossless) |
| `log` | | structured logging via the `log` facade |

Build with `default-features = false` for a dependency-free type system that still
parses, renders and compares schemas.
