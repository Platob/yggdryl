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
  child element of every nested type.

Every type pairs a canonical-string `from_str` / `to_str` round-trip with, under
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
