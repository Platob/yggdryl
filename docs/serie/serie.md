# Serie

A `Serie` is a single named, **typed column** — the layer between the
[schema](../schema/datatype.md) type system and a future dataframe. It pairs a
[`Field`](../schema/field.md) (name + `DataType` + nullability + metadata) with an
Apache **Arrow** array holding the values, so a column carries both its logical type
and its physical storage.

!!! note "Rust core first"
    `yggdryl-serie` is the Arrow-backed foundation a `Frame` / `LazyFrame` /
    `ParquetFrame` will build on. The examples below are the Rust API; the **Python
    and Node bindings are planned** and this page will gain synced language tabs once
    they land.

## The model

The design mirrors the schema crate's three [categories](../schema/datatype.md):

- **`Serie`** — the object-safe base trait every column implements: accessors to the
  `field()` and the backing Arrow `array()`, the `len()` / `null_count()` / `is_null()`
  bookkeeping, `slice()` and downcasting via `as_any()`.
- **`TypedSerie<T>`** — typed value access (`get` / `value` / `iter` / `to_vec`) over a
  column's native value type `T`.
- The **primitive** concrete series — `PrimitiveSerie<A>` (every Arrow numeric, decimal
  and temporal type), `BooleanSerie`, `VarcharSerie<O>` and `BinarySerie<O>`. Named
  aliases (`Int32Serie`, `Float64Serie`, `TimestampMicrosecondSerie`, …) pin the common
  widths.

## Build a column

`from_array` derives the field from the Arrow type; `from_arrow` takes an explicit
`Field` (carrying name, nullability and metadata). Both **redirect** the array to the
right concrete series and return a boxed `SerieRef`.

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, from_arrow, Field, DataType, Serie, TypedSerie, Int32Serie};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
    use std::sync::Arc;

    // derive the field from the array
    let array: ArrayRef = Arc::new(Int32Array::from(vec![Some(1), None, Some(3)]));
    let serie = from_array("id", array)?;
    assert_eq!(serie.len(), 3);
    assert_eq!(serie.null_count(), 1);
    assert_eq!(serie.data_type(), &DataType::int(32, true));

    // or supply a field with metadata / nullability
    let field = Field::new("id", DataType::int(32, true), false).with_comment("primary key");
    let serie = from_arrow(field, Arc::new(Int32Array::from(vec![1, 2, 3])))?;
    assert!(!serie.is_nullable());
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Typed access

Recover the concrete series with `as_any().downcast_ref()`, then read values through
`TypedSerie<T>` — `get` returns `None` for null or out-of-bounds, `value` panics on
those, `iter` / `to_vec` yield `Option<T>`.

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, TypedSerie, Int32Serie, VarcharSerie};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array, StringArray};
    use std::sync::Arc;

    let serie = from_array("n", Arc::new(Int32Array::from(vec![Some(1), None, Some(3)])) as ArrayRef)?;
    let ints = serie.as_any().downcast_ref::<Int32Serie>().unwrap();
    assert_eq!(ints.get(0), Some(1));
    assert_eq!(ints.get(1), None);
    assert_eq!(ints.value(2), 3);
    assert_eq!(ints.to_vec(), vec![Some(1), None, Some(3)]);

    // strings expose a zero-copy `str_value` alongside the owned `get`
    let serie = from_array("s", Arc::new(StringArray::from(vec![Some("a"), None])) as ArrayRef)?;
    let strings = serie.as_any().downcast_ref::<VarcharSerie<i32>>().unwrap();
    assert_eq!(strings.str_value(0), Some("a"));
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

Build columns directly from values without an Arrow array:

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, VarcharSerie, BooleanSerie, TypedSerie};

    let ints = Int32Serie::from_values("n", vec![Some(1), None, Some(3)]);
    let names = VarcharSerie::<i32>::from_values("name", vec![Some("a"), Some("b")]);
    let flags = BooleanSerie::from_values("ok", vec![Some(true), Some(false), None]);
    assert_eq!(ints.get(1), None);
    assert_eq!(names.str_value(0), Some("a"));
    assert_eq!(flags.null_count(), 1);
    ```

## Slice & inspect

`slice` is a zero-copy view of the same type; the base accessors report the column's
name, type, category and null layout.

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, Serie, Int32Serie, TypedSerie};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
    use std::sync::Arc;

    let serie = from_array("n", Arc::new(Int32Array::from(vec![10, 20, 30, 40])) as ArrayRef)?;
    let window = serie.slice(1, 2);
    assert_eq!(window.len(), 2);
    assert_eq!(window.name(), "n");
    let typed = window.as_any().downcast_ref::<Int32Serie>().unwrap();
    assert_eq!(typed.value(0), 20);
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Coverage

The primitive category is complete: integers (`int8`…`int64`, `uint8`…`uint64`),
floats (`float16`/`32`/`64`), decimals (128/256), every temporal physical type
(date/time/timestamp/duration/interval), boolean, UTF-8 strings (`Utf8` / `LargeUtf8`)
and binary (`Binary` / `LargeBinary`). The **nested** (list / struct / map / union),
**dictionary** and **view** backends, a **`ChunkedSerie`** mirroring Arrow's
`ChunkedArray`, and cast / arithmetic operations are the next increments.

## Next

- [DataType](../schema/datatype.md) — the logical type a serie carries
- [Field](../schema/field.md) — naming a column, building a schema
