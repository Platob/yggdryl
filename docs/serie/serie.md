# Serie

A `Serie` is a single named, **typed column** — the layer between the
[schema](../schema/datatype.md) type system and a future dataframe. It pairs a
[`Field`](../schema/field.md) (name + `DataType` + nullability + metadata) with an
Apache **Arrow** array holding the values, so a column carries both its logical type
and its physical storage. Columns can also be **lazy** (computed on demand) or
**children** (zero-copy slices that remember their parent).

!!! note "Rust core first"
    `yggdryl-serie` is the Arrow-backed foundation a `Frame` / `LazyFrame` /
    `ParquetFrame` will build on. The examples below are the Rust API; the **Python
    and Node bindings are planned** and this page will gain synced language tabs once
    they land.

## The model

The design mirrors the schema crate's three [categories](../schema/datatype.md):

- **`Serie`** — the object-safe base trait every column implements: accessors to the
  `field()` and the backing Arrow `array()`; the `len()` / `num_rows()` / `null_count()`
  / `metadata()` bookkeeping; value access by index (`value_at` → `Scalar`) and by range
  (`slice` / `slice_range`, zero-copy); the `parent()` graph link; `materialize()`; and
  downcasting via `as_any()`.
- **`TypedSerie<T>`** — typed value access (`get` / `value` / `iter` / `to_vec`) over a
  column's native value type `T`.
- The **primitive** concrete series — `PrimitiveSerie<A>` (every Arrow numeric, decimal
  and temporal type), `BooleanSerie`, `VarcharSerie<O>` and `BinarySerie<O>`. Named
  aliases (`Int32Serie`, `Float64Serie`, `TimestampMicrosecondSerie`, …) pin the common
  widths.
- The **lazy** (computed) series — `RangeSerie` and `DateRangeSerie`.
- **`IndexSerie`** — a row index, defaulting to a lazy `uint64` range.

## Build a column

`from_array` derives the field from the Arrow type; `from_arrow` takes an explicit
`Field` (carrying name, nullability and metadata). Both **redirect** the array to the
right concrete series and return a boxed `SerieRef`.

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, from_arrow, Field, DataType, Serie};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
    use std::sync::Arc;

    let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(3)]));
    let serie = from_array("id", array)?;
    assert_eq!(serie.num_rows(), 3);
    assert_eq!(serie.null_count(), 1);
    assert_eq!(serie.data_type(), &DataType::int(32, true));

    let field = Field::new("id", DataType::int(32, true), false).with_comment("primary key");
    let serie = from_arrow(field, Arc::new(Int32Array::from(vec![1, 2, 3])))?;
    assert!(!serie.is_nullable());
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Values: by index and by range

`value_at` reads a single cell as a type-erased `Scalar` (`Null` for a null or
out-of-bounds cell); `slice` / `slice_range` return a **zero-copy** sub-column. For
typed access, downcast to the concrete series and use `TypedSerie<T>`.

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, Serie, Scalar, Int32Serie, TypedSerie};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
    use std::sync::Arc;

    let serie = from_array("n", Arc::new(Int32Array::from(vec![Some(5), None, Some(7)])) as ArrayRef)?;

    // by index → Scalar
    assert_eq!(serie.value_at(0), Scalar::Int(5));
    assert_eq!(serie.value_at(1), Scalar::Null);   // null cell
    assert_eq!(serie.value_at(9), Scalar::Null);   // out of bounds

    // by range → zero-copy slice
    let window = serie.slice_range(1..3);
    assert_eq!(window.len(), 2);

    // typed access through a downcast
    let ints = serie.as_any().downcast_ref::<Int32Serie>().unwrap();
    assert_eq!(ints.get(0), Some(5));
    assert_eq!(ints.value(2), 7);
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## The slice graph: children & parents

`child` (and `child_range`) build a zero-copy slice that remembers the serie it came
from via `parent()` — a navigable graph. `materialize()` realises a column into an
independent, in-memory one and **detaches** it from the graph.

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, child, Serie, Scalar};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
    use std::sync::Arc;

    let parent = from_array("n", Arc::new(Int32Array::from(vec![10, 20, 30, 40])) as ArrayRef)?;
    let view = child(&parent, 1, 2);                 // rows 1..3, linked to parent

    assert_eq!(view.value_at(0), Scalar::Int(20));
    assert_eq!(view.parent().unwrap().num_rows(), 4); // walk back up

    let independent = view.materialize();             // detach from the graph
    assert!(independent.parent().is_none());
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Lazy (computed) series

A lazy column stores only a compact description and computes its values on demand
(`is_materialized()` is `false`) until `materialize()` realises a real Arrow array.

- `RangeSerie` — a `uint64` arithmetic range `start, start+step, …`.
- `DateRangeSerie` — a day-resolution calendar-date range (`Date32`).

=== "Rust"

    ```rust
    use yggdryl_serie::{RangeSerie, DateRangeSerie, Serie, TypedSerie, Scalar};
    use yggdryl_core::Date;

    let r = RangeSerie::new("r", 100, 5, 4);          // 100, 105, 110, 115 (lazy)
    assert!(!r.is_materialized());
    assert_eq!(r.get(2), Some(110));
    assert_eq!(r.value_at(3), Scalar::Int(115));
    let realized = r.materialize();                    // -> a real uint64 column
    assert!(realized.is_materialized());

    let dates = DateRangeSerie::from_dates("d", Date::from_ymd(2024, 1, 30).unwrap(), 1, 3);
    assert_eq!(dates.date_at(2), Some(Date::from_ymd(2024, 2, 1).unwrap()));
    ```

## Index

`IndexSerie` is the row index — a `Serie` of labels with label ↔ position lookups. The
default is a **lazy** `uint64` range; any column can be wrapped as an index.

=== "Rust"

    ```rust
    use yggdryl_serie::{IndexSerie, Serie};

    let index = IndexSerie::range(4);                 // lazy [0, 1, 2, 3] (uint64)
    assert!(index.is_range());
    assert!(!index.is_materialized());
    assert_eq!(index.at(2), Some(2));                 // label at row 2
    assert_eq!(index.position(3), Some(3));           // row of label 3
    assert!(!index.contains(4));
    ```

## Coverage

The primitive category is complete: integers (`int8`…`int64`, `uint8`…`uint64`),
floats (`float16`/`32`/`64`), decimals (128/256), every temporal physical type
(date/time/timestamp/duration/interval), boolean, UTF-8 strings (`Utf8` / `LargeUtf8`)
and binary (`Binary` / `LargeBinary`), plus the lazy `RangeSerie` / `DateRangeSerie`,
`IndexSerie` and the `SliceSerie` graph. The **nested** (list / struct / map / union),
**dictionary** and **view** backends, a **`ChunkedSerie`** mirroring Arrow's
`ChunkedArray`, cast / arithmetic operations, and the **Python / Node bindings** are the
next increments.

## Next

- [DataType](../schema/datatype.md) — the logical type a serie carries
- [Field](../schema/field.md) — naming a column, building a schema
