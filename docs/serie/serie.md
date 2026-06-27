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

- **`Serie`** — the object-safe base trait every column implements: convenience field
  reflections (`name()`, `dtype()`, `get_metadata(key)`); accessors to the `field()` and
  the backing Arrow `array()`; the `len()` / `num_rows()` / `null_count()` bookkeeping;
  value access by index (`value_at` → `Scalar`) and by range (`slice` / `slice_range`,
  zero-copy); the `parent()` graph link; `materialize()`; a parametrised `display()`; and
  downcasting via `as_any()`.
- **`TypedSerie<T>`** — typed value access (`get` / `value` / `iter` / `to_vec`) over a
  column's native value type `T`.
- The **primitive** concrete series — `PrimitiveSerie<A>` (Arrow numeric / date / interval
  types), `BooleanSerie`, `VarcharSerie<O>` and `BinarySerie<O>`, with named aliases
  (`Int32Serie`, `Float64Serie`, `Date32Serie`, …).
- The **temporal** series — `DatetimeSerie`, `TimeSerie` and `DurationSerie` (unified
  columns over any unit, presenting core `DateTime` / `Time` / `Duration`) and the
  `TemporalSerie` trait (`datetime_at` / `date_at` / `time_at`).
- The **nested** series — `StructSerie`, `ListSerie<O>` and `MapSerie` (child columns
  built recursively) and the `NestedSerie` trait.
- The **lazy** (computed) series — `RangeSerie`, `DateRangeSerie`, `DateTimeRangeSerie`,
  `TimeRangeSerie`.
- **`IndexSerie`** — a row index, defaulting to a lazy `uint64` range.
- **`EnumSerie`** — a categorical view mapping unique values to a code and first row.

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

## Creating series — one line per type

Every concrete series has a `from_values(name, values)` one-liner; nested and lazy
columns have purpose-built constructors. `from_array(name, arrow_array)` is the
universal fallback for *any* Arrow array.

=== "Rust"

    ```rust
    use yggdryl_serie::{
        Int32Serie, Float64Serie, BooleanSerie, VarcharSerie, BinarySerie,
        DatetimeSerie, TimeSerie, DurationSerie, DateRangeSerie, RangeSerie,
        IndexSerie, StructSerie, EnumSerie, Serie, SerieRef, TypedSerie,
    };
    use yggdryl_core::{DateTime, Duration, Time, Date};
    use std::sync::Arc;

    // primitives
    let i = Int32Serie::from_values("i", vec![Some(1), None, Some(3)]);
    let f = Float64Serie::from_values("f", vec![Some(1.5), Some(2.5)]);
    let b = BooleanSerie::from_values("b", vec![Some(true), Some(false)]);
    let s = VarcharSerie::<i32>::from_values("s", vec![Some("a"), Some("b")]);
    let bin = BinarySerie::<i32>::from_values("raw", vec![Some(&b"xy"[..])]);

    // temporal — values are the core DateTime / Time / Duration
    let ts = DatetimeSerie::from_values("ts", vec![Some(DateTime::from_epoch_seconds(0, None))]);
    let tm = TimeSerie::from_values("t", vec![Some(Time::from_hms(9, 30, 0).unwrap())]);
    let du = DurationSerie::from_values("d", vec![Some(Duration::from_secs(60))]);

    // lazy ranges + index (computed, not stored)
    let r = RangeSerie::new("r", 0, 1, 100);            // 0..100 (uint64)
    let days = DateRangeSerie::from_dates("d", Date::from_ymd(2024, 1, 1).unwrap(), 1, 7);
    let idx = IndexSerie::range(100);

    // nested — a struct from its child columns, in one line
    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("name", vec![Some("a"), Some("b")]));
    let rec = StructSerie::from_children("rec", vec![id, name])?;

    // categorical view over any column
    let cat = EnumSerie::from_serie(Arc::new(BooleanSerie::from_values("c", vec![Some(true), Some(false), Some(true)])));
    assert_eq!(rec.child_count(), 2);
    assert_eq!(cat.unique_count(), 2);
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

Lists and maps are built from an Arrow builder and wrapped with `from_array`:

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, ListSerie, Serie, Scalar};
    use yggdryl_serie::arrow_array::{ArrayRef, ListArray};
    use yggdryl_serie::arrow_array::types::Int32Type;
    use std::sync::Arc;

    let lists = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![
        Some(vec![Some(1), Some(2)]), Some(vec![Some(3)]),
    ]);
    let serie = from_array("l", Arc::new(lists) as ArrayRef)?;
    assert_eq!(serie.value_at(0), Scalar::Other("[1, 2]".into()));
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Defaults & resize

Every datatype has a default value (`Scalar::default_for`): `false`, `0`, `0.0`, the
empty string, empty bytes, a struct of defaults. `resize(new_len)` slices when
shrinking and extends when growing — with **nulls** if the column is nullable, otherwise
the type **default** (so a non-nullable column never gains a null).

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, Scalar, Serie, DataType, Field, from_arrow};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
    use std::sync::Arc;

    assert_eq!(Scalar::default_for(&DataType::int(32, true)), Scalar::Int(0));
    assert_eq!(Scalar::default_for(&DataType::varchar()), Scalar::Utf8(String::new()));

    // nullable column → grows with nulls
    let nullable = Int32Serie::from_values("n", vec![Some(1), Some(2)]);
    assert_eq!(nullable.resize(4)?.value_at(3), Scalar::Null);

    // non-nullable column → grows with the type default (0)
    let strict = from_arrow(Field::new("n", DataType::int(32, true), false),
                            Arc::new(Int32Array::from(vec![7])) as ArrayRef)?;
    assert_eq!(strict.resize(3)?.value_at(2), Scalar::Int(0));
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

    // null / presence checks (out-of-bounds reads as null)
    assert!(serie.is_null(1));
    assert!(serie.is_valid(0));
    assert!(!serie.is_empty());

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
- `DateTimeRangeSerie` — a nanosecond timestamp range.
- `TimeRangeSerie` — a time-of-day range (wraps within the day).

The three temporal ranges implement `TemporalSerie` (see below).

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

## Temporal series

`DatetimeSerie` is the **unified timestamp column**: it backs any unit (second …
nanosecond) and an optional timezone, exposing values as the core `DateTime`. `TimeSerie`
and `DurationSerie` are its time-of-day and elapsed-time counterparts — each unifies every
unit and presents core `Time` / `Duration`, respecting the `Time{unit}` / `Duration{unit}`
data types (so there are no per-unit aliases). Every timestamp / time / duration array
dispatches to these. All temporal columns (including the date/time/datetime ranges)
implement `TemporalSerie` — a uniform `datetime_at` with derived `date_at` / `time_at`
(`DurationSerie` is a span, so it is not `TemporalSerie`).

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, DatetimeSerie, DateTimeRangeSerie, TimeRangeSerie, TemporalSerie, Serie};
    use yggdryl_serie::arrow_array::{ArrayRef, TimestampMicrosecondArray};
    use yggdryl_core::{DateTime, Duration, Time};
    use std::sync::Arc;

    // a timestamp array becomes a DatetimeSerie (any unit + tz)
    let ts = from_array("ts", Arc::new(TimestampMicrosecondArray::from(vec![10, 20])) as ArrayRef)?;
    let dt = ts.as_any().downcast_ref::<DatetimeSerie>().unwrap();
    assert_eq!(dt.datetime_at(0).unwrap().epoch_nanos(), 10_000); // 10µs

    // lazy temporal ranges, all TemporalSerie
    let hours = DateTimeRangeSerie::new("h", &DateTime::from_epoch_seconds(0, None), &Duration::from_secs(3600), 3);
    assert_eq!(hours.datetime_at(2).unwrap().epoch_seconds(), 7200);

    let clock = TimeRangeSerie::new("t", Time::from_hms(23, 0, 0).unwrap(), Duration::from_secs(3600), 3);
    assert_eq!(clock.time(1), Time::from_hms(0, 0, 0).ok()); // wraps past midnight
    # Ok::<(), yggdryl_serie::SerieError>(())
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

Slicing a range index drops the range fast-path flag (the labels no longer start at
`0`), but the result is still an `IndexSerie` and `at` / `position` keep working.

## Enum (categorical)

`EnumSerie` scans a column once and holds the mapping of unique values to a compact
**code** and to their **first row index** — the basis for a categorical/dictionary
column.

=== "Rust"

    ```rust
    use yggdryl_serie::{EnumSerie, VarcharSerie, Scalar, Serie};
    use std::sync::Arc;

    let values = VarcharSerie::<i32>::from_values("c", vec![Some("a"), Some("b"), Some("a")]);
    let enums = EnumSerie::from_serie(Arc::new(values));
    assert_eq!(enums.unique_count(), 2);                          // "a", "b"
    assert_eq!(enums.code(&Scalar::Utf8("b".into())), Some(1));   // enum code
    assert_eq!(enums.first_row(&Scalar::Utf8("a".into())), Some(0));
    assert_eq!(enums.code_at(2), Some(0));                        // row 2 holds "a"
    ```

## Nested

`StructSerie`, `ListSerie<O>` and `MapSerie` are columns of columns. Each builds its
child [`Serie`]s **recursively** through the same factory, so arbitrarily deep nesting (a
list of structs of maps, …) resolves uniformly. The `NestedSerie` trait exposes
`child_count` / `child(index)` / `children` / `child_by_name`.

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, NestedSerie, ListSerie, Scalar, Serie};
    use yggdryl_serie::arrow_array::{ArrayRef, ListArray};
    use yggdryl_serie::arrow_array::types::Int32Type;
    use std::sync::Arc;

    // list<int32>: [[1, 2], [3], null]
    let la = ListArray::from_iter_primitive::<Int32Type, _, _>(vec![
        Some(vec![Some(1), Some(2)]), Some(vec![Some(3)]), None,
    ]);
    let serie = from_array("l", Arc::new(la) as ArrayRef)?;
    let list = serie.as_any().downcast_ref::<ListSerie<i32>>().unwrap();

    assert_eq!(list.value_slice(0).unwrap().len(), 2);   // the sub-list is a zero-copy Serie
    assert!(list.value_slice(2).is_none());              // null row
    assert_eq!(list.value_at(1), Scalar::Other("[3]".into()));
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

### Child access — by index, name or path

Any column exposes `select("a.b.c")` to navigate into nested children, and
`as_nested()` for the full child API (by index, or by name with a case-sensitive →
case-insensitive fallback). A path segment may be **wrapped** (`[name]`, `"name"`,
`'name'`, `` `name` ``) to match the literal name exactly (and to contain dots); a bare
numeric segment is a child index.

=== "Rust"

    ```rust
    use yggdryl_serie::{StructSerie, Int32Serie, VarcharSerie, NestedSerie, Serie, SerieRef, Scalar};
    use std::sync::Arc;

    let inner: SerieRef = Arc::new(StructSerie::from_children("inner", vec![
        Arc::new(Int32Serie::from_values("a", vec![Some(1), Some(2)])) as SerieRef,
    ]).unwrap());
    let label: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("Label", vec![Some("x"), Some("y")]));
    let rec: SerieRef = Arc::new(StructSerie::from_children("rec", vec![inner, label]).unwrap());

    assert_eq!(rec.select("inner.a").unwrap().value_at(1), Scalar::Int(2)); // path
    assert_eq!(rec.select("label").unwrap().name(), "Label");               // case-insensitive
    assert_eq!(rec.select("[inner].a").unwrap().value_at(0), Scalar::Int(1)); // wrapped exact

    let nested = rec.as_nested().unwrap();
    assert_eq!(nested.child(0).unwrap().name(), "inner"); // by index
    assert_eq!(nested.children().len(), 2);
    ```

## Display

`Serie::display(&DisplayOptions)` renders a column to a readable string — the building
block for a future `Frame`'s table. Parameters: `max_rows`, `header`, `width`, `null` and
`index`.

=== "Rust"

    ```rust
    use yggdryl_serie::{DisplayOptions, Serie, Int32Serie};

    let serie = Int32Serie::from_values("n", (0..100).map(Some));
    let text = serie.display(&DisplayOptions::default().with_max_rows(3));
    assert!(text.contains("n: int32"));        // header
    assert!(text.contains("97 more rows"));    // truncation marker
    ```

## Coverage

The primitive category is complete: integers (`int8`…`int64`, `uint8`…`uint64`),
floats (`float16`/`32`/`64`), decimals (128/256), dates and intervals, boolean, UTF-8
strings (`Utf8` / `LargeUtf8`) and binary (`Binary` / `LargeBinary`); timestamps, times
and durations unify into `DatetimeSerie` / `TimeSerie` / `DurationSerie`. The **nested**
`StructSerie` / `ListSerie` / `MapSerie` (recursive), the lazy `RangeSerie` /
`DateRangeSerie` / `DateTimeRangeSerie` / `TimeRangeSerie`, the `IndexSerie`, `EnumSerie`,
the `TemporalSerie` / `NestedSerie` traits, the `SliceSerie` graph and `display()` round
it out. The **union** nested type, the **dictionary** and **view** backends, a
**`ChunkedSerie`** mirroring Arrow's `ChunkedArray`, cast / arithmetic operations,
**benchmarks** and the **Python / Node bindings** are the next increments.

## Next

- [DataType](../schema/datatype.md) — the logical type a serie carries
- [Field](../schema/field.md) — naming a column, building a schema
