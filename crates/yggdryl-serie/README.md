# yggdryl-serie

Arrow-backed **columnar series** for yggdryl — the layer between the
[`yggdryl-schema`](../yggdryl-schema) type system and a future dataframe.

A `Serie` is a single named, typed column: a `Field` (name + `DataType` +
nullability + metadata) paired with an Apache Arrow array holding the values. The
design mirrors the schema crate's three categories:

- `Serie` — the object-safe base trait every column implements (field + backing
  array accessors, length / null bookkeeping, `slice`, downcasting).
- `TypedSerie<T>` — typed value access (`get` / `value` / `iter`) over a column's
  native value type.
- The **primitive** concrete series: `PrimitiveSerie<A>` (every Arrow numeric,
  decimal and temporal type), `BooleanSerie`, `VarcharSerie<O>` and `BinarySerie<O>`,
  with named aliases (`Int32Serie`, `Float64Serie`, `TimestampMicrosecondSerie`, …).

`from_arrow` / `from_array` redirect an Arrow array to the right concrete series.

```rust
use yggdryl_serie::{from_array, Int32Serie, TypedSerie};
use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
use std::sync::Arc;

let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(3)]));
let serie = from_array("id", array).unwrap();
assert_eq!(serie.len(), 3);
assert_eq!(serie.null_count(), 1);

let ints = serie.as_any().downcast_ref::<Int32Serie>().unwrap();
assert_eq!(ints.get(0), Some(1));
```

See the [docs site](https://platob.github.io/yggdryl/) for the full guide.
