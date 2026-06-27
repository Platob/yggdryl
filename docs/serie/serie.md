# Serie

A `Serie` is a single named, **typed column** — the layer between the
[schema](../schema/datatype.md) type system and a future dataframe. It pairs a
[`Field`](../schema/field.md) (name + `DataType` + nullability + metadata) with an
Apache **Arrow** array holding the values, so a column carries both its logical type
and its physical storage. Columns can also be **lazy** (computed on demand) or
**children** (zero-copy slices that remember their parent).

!!! note "Available in all three languages"
    `yggdryl-serie` is the Arrow-backed columnar foundation. A **struct column is a
    [DataFrame](#frame-dataframe)** (its children are the columns), so the same `Serie`
    class is both the column *and* the table. The API is surfaced in **Python** and
    **Node** as a single `Serie` class — build from a list, read / update by index, slice
    / resize / cast, navigate nested children, run frame ops (select / filter / sort /
    stack / records), round-trip through bytes — as well as the Rust core, which also
    exposes the richer concrete-series internals (typed downcasts, the slice graph)
    directly.

=== "Python"

    ```python
    import yggdryl

    s = yggdryl.Serie("n", [1, None, 3])
    assert len(s) == 3 and s.null_count == 1
    assert str(s.data_type) == "int64"
    assert s[0] == 1 and s[1] is None
    assert s.cast("float64")[0] == 1.0
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const s = new Serie('n', [1, null, 3])
    // numbers infer int64 when all integral (JS has no int type)
    if (s.numRows !== 3 || s.nullCount !== 1) throw new Error('shape')
    if (s.dataType.toString() !== 'int64') throw new Error('type')
    if (s.get(0) !== 1 || s.get(1) !== null) throw new Error('values')
    if (s.cast('float64').get(0) !== 1) throw new Error('cast')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, Scalar, Serie};

    let s = Int32Serie::from_values("n", vec![Some(1), None, Some(3)]);
    assert_eq!(s.num_rows(), 3);
    assert_eq!(s.null_count(), 1);
    assert_eq!(s.value_at(0), Scalar::Int(1));
    ```

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
- **`CategoricalSerie`** — a dictionary-encoded view for repeated values (distinct values
  + a per-row code), decoding back to a flat column on `materialize`.

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

In the bindings a single `Serie` builds any column from a list (the type is inferred,
or pass a `dtype`), with `range` / `index` / `struct` / `binary` factories for the rest.
In Rust each concrete series has a `from_values(name, values)` one-liner; `from_array`
is the universal fallback for *any* Arrow array.

=== "Python"

    ```python
    import yggdryl

    ints = yggdryl.Serie("i", [1, None, 3])                 # int64
    floats = yggdryl.Serie("f", [1.5, 2.5])                 # float64
    flags = yggdryl.Serie("b", [True, False])               # bool
    text = yggdryl.Serie("s", ["a", "b"])                   # utf8
    small = yggdryl.Serie("i8", [1, 2, 3], dtype="int8")    # explicit dtype

    rng = yggdryl.Serie.range(100)                          # lazy 0..100
    idx = yggdryl.Serie.index(100)                          # lazy row index
    rec = yggdryl.Serie.struct("rec", [                     # nested, one line
        yggdryl.Serie("id", [1, 2]),
        yggdryl.Serie("name", ["a", "b"]),
    ])
    cat = yggdryl.Serie("c", ["a", "b", "a"]).categorical() # dictionary-encoded
    assert rec.children()[0].name == "id"
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const ints = new Serie('i', [1, null, 3])               // int64 (all integral)
    const floats = new Serie('f', [1.5, 2.5])               // float64
    const flags = new Serie('b', [true, false])             // bool
    const text = new Serie('s', ['a', 'b'])                 // utf8
    const small = new Serie('i8', [1, 2, 3], 'int8')        // explicit dtype

    const rng = Serie.range(100)                            // lazy 0..100
    const idx = Serie.index(100)                            // lazy row index
    const rec = Serie.struct('rec', [                       // nested, one line
      new Serie('id', [1, 2]),
      new Serie('name', ['a', 'b']),
    ])
    const cat = new Serie('c', ['a', 'b', 'a']).categorical() // dictionary-encoded
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{
        Int32Serie, Float64Serie, BooleanSerie, VarcharSerie, BinarySerie,
        DatetimeSerie, TimeSerie, DurationSerie, DateRangeSerie, RangeSerie,
        IndexSerie, StructSerie, CategoricalSerie, Serie, SerieRef, TypedSerie,
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

    // categorical (dictionary-encoded) view over any column
    let cat = CategoricalSerie::from_serie(
        &BooleanSerie::from_values("c", vec![Some(true), Some(false), Some(true)]),
    )?;
    assert_eq!(rec.child_count(), 2);
    assert_eq!(cat.category_count(), 2);
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

## Categorical (dictionary-encoded)

`CategoricalSerie` is a **dictionary-encoded** view for *repeated values*: it stores the
distinct values once plus a compact per-row code, so a low-cardinality column is held
compactly. It is lazy (`is_materialized()` is `false`); `materialize()` decodes it back
into a flat column.

=== "Rust"

    ```rust
    use yggdryl_serie::{CategoricalSerie, VarcharSerie, Scalar, Serie};

    let values = VarcharSerie::<i32>::from_values("c", vec![Some("a"), Some("b"), Some("a")]);
    let cat = CategoricalSerie::from_serie(&values).unwrap();
    assert_eq!(cat.category_count(), 2);              // "a", "b" stored once
    assert_eq!(cat.code_at(0), cat.code_at(2));       // repeated "a" shares a code
    assert_eq!(cat.value_at(1), Scalar::Utf8("b".into()));
    assert!(!cat.is_materialized());

    let flat = cat.materialize();                     // decode -> a real varchar column
    assert!(flat.is_materialized());
    ```

## Cast

`cast(dtype)` converts a column's values (Arrow's cast kernel — including lossy /
narrowing casts, which yield null on overflow). A **struct → struct** cast matches
children by name, casts each, **fills missing** target columns (null if nullable, else
the type default) and drops extras. `dtype` is a `DataType` or a type string.

=== "Python"

    ```python
    import yggdryl

    ints = yggdryl.Serie("n", [1, 2, 3])
    assert ints.cast("float64")[0] == 1.0
    big = yggdryl.Serie("n", [1000, 5]).cast("int8")    # narrowing
    assert big[0] is None                               # 1000 overflows int8
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const ints = new Serie('n', [1, 2, 3])
    if (ints.cast('float64').get(0) !== 1) throw new Error('cast')
    const big = new Serie('n', [1000, 5]).cast('int8')  // narrowing
    if (big.get(0) !== null) throw new Error('overflow') // 1000 overflows int8
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, Int32Serie, StructSerie, DataType, Field, NestedSerie, Serie, Scalar, SerieRef};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
    use std::sync::Arc;

    // primitive cast (lossy narrowing yields null on overflow)
    let ints = from_array("n", Arc::new(Int32Array::from(vec![1, 2, 3])) as ArrayRef)?;
    assert_eq!(ints.cast(&DataType::float(64))?.value_at(0), Scalar::Float(1.0));

    // struct cast with a missing column filled
    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let rec: SerieRef = Arc::new(StructSerie::from_children("rec", vec![id])?);
    let target = DataType::struct_(vec![
        Field::new("id", DataType::int(64, true), true),    // widened
        Field::new("extra", DataType::varchar(), true),     // missing -> filled null
    ]);
    let casted = rec.cast(&target)?;
    assert_eq!(casted.as_nested().unwrap().child_by_name("extra").unwrap().value_at(0), Scalar::Null);
    # Ok::<(), yggdryl_serie::SerieError>(())
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
numeric segment is a child index. The path is **parsed first**, so `select` returns
`Result<Option<…>>`: a malformed path (unclosed wrapper, empty segment) is an error,
while a well-formed path that does not resolve — a missing child, or a leaf column — is
`Ok(None)` (`None` in the bindings); a malformed path raises.

=== "Python"

    ```python
    import yggdryl

    rec = yggdryl.Serie.struct("rec", [
        yggdryl.Serie.struct("inner", [yggdryl.Serie("a", [1, 2])]),
        yggdryl.Serie("Label", ["x", "y"]),
    ])
    assert rec.select("inner.a")[1] == 2          # node path
    assert rec.select("label").name == "Label"    # case-insensitive
    assert rec.child(0).name == "inner"           # by index
    assert rec.select("inner.zzz") is None        # unresolved
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const rec = Serie.struct('rec', [
      Serie.struct('inner', [new Serie('a', [1, 2])]),
      new Serie('Label', ['x', 'y']),
    ])
    if (rec.select('inner.a').get(1) !== 2) throw new Error('path')
    if (rec.select('label').name !== 'Label') throw new Error('ci')
    if (rec.child(0).name !== 'inner') throw new Error('index')
    if (rec.select('inner.zzz') !== null) throw new Error('unresolved')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{StructSerie, Int32Serie, VarcharSerie, NestedSerie, Serie, SerieRef, Scalar};
    use std::sync::Arc;

    let inner: SerieRef = Arc::new(StructSerie::from_children("inner", vec![
        Arc::new(Int32Serie::from_values("a", vec![Some(1), Some(2)])) as SerieRef,
    ]).unwrap());
    let label: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("Label", vec![Some("x"), Some("y")]));
    let rec: SerieRef = Arc::new(StructSerie::from_children("rec", vec![inner, label]).unwrap());

    // select returns Result<Option<…>>: Ok(Some) found, Ok(None) unresolved, Err malformed
    assert_eq!(rec.select("inner.a")?.unwrap().value_at(1), Scalar::Int(2)); // path
    assert_eq!(rec.select("label")?.unwrap().name(), "Label");               // case-insensitive
    assert_eq!(rec.select("[inner].a")?.unwrap().value_at(0), Scalar::Int(1)); // wrapped exact
    assert!(rec.select("inner.zzz")?.is_none());                             // unresolved
    assert!(rec.select("inner.").is_err());                                  // malformed path

    let nested = rec.as_nested().unwrap();
    assert_eq!(nested.child(0).unwrap().name(), "inner"); // by index
    assert_eq!(nested.children().len(), 2);
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Frame (DataFrame)

**A struct column *is* a DataFrame** — its child columns *are* the frame's columns, so
`StructSerie` carries the table surface directly (there is no separate `Frame` type).
Build one with the `struct` factory, then project / filter / sort / stack rows and read
records back, with a pandas-like feel: every transform is **functional** and returns a new
lazy frame that **shares the untouched columns' Arrow buffers** (no copy), assembling the
backing `StructArray` only on demand.

=== "Python"

    ```python
    import yggdryl

    df = yggdryl.Serie.struct("df", [
        yggdryl.Serie("id", [3, 1, 2]),
        yggdryl.Serie("name", ["c", "a", "b"]),
    ])
    assert df.shape == (3, 2)                            # (rows, columns)
    assert df.column_names == ["id", "name"]

    # columns: project / add / drop / rename (all return a new frame)
    df.select_columns(["name"])                         # keep / reorder a subset
    df.with_column(yggdryl.Serie("ok", [True, True, False]))
    df.drop_columns(["name"])
    df.rename("id", "key")

    # rows: filter / sort / stack / row-index / head / tail
    df.filter([True, False, True])
    df.vstack(df)
    df.with_row_index("i")                              # prepend a 0..n column

    # read rows back as native records
    assert df.row(1).to_dict() == {"id": 1, "name": "a"}
    row = df.row(1).as_dataclass("Row")                 # a real dataclass instance
    assert (row.id, row.name) == (1, "a")
    assert df.sort_by("id").to_dicts() == [
        {"id": 1, "name": "a"}, {"id": 2, "name": "b"}, {"id": 3, "name": "c"},
    ]

    print(df.display(max_rows=10))                      # an aligned table
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const df = Serie.struct('df', [
      new Serie('id', [3, 1, 2]),
      new Serie('name', ['c', 'a', 'b']),
    ])
    if (df.shape[0] !== 3 || df.shape[1] !== 2) throw new Error('shape')
    if (df.columnNames.join() !== 'id,name') throw new Error('columns')

    // columns
    df.selectColumns(['name'])
    df.withColumn(new Serie('ok', [true, true, false]))
    df.dropColumns(['name'])
    df.rename('id', 'key')

    // rows
    df.filter([true, false, true])
    df.vstack(df)
    df.withRowIndex('i')

    // records
    const rec = df.row(1).toObject()                    // { id: 1, name: 'a' }
    const rows = df.sortBy('id').toDicts()
    console.log(df.display(10))
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, VarcharSerie, StructSerie, Serie, SerieRef, DisplayOptions};
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(3), Some(1), Some(2)]));
    let name: SerieRef = Arc::new(VarcharSerie::<i32>::from_values("name", vec![Some("c"), Some("a"), Some("b")]));
    let df = StructSerie::from_children("df", vec![id, name])?;

    assert_eq!(df.shape(), (3, 2));
    assert_eq!(df.column_names(), vec!["id", "name"]);

    // columns
    let _ = df.select_columns(&["name"])?;
    let _ = df.drop_columns(&["name"])?;
    let _ = df.rename("id", "key")?;

    // rows
    let asc = df.sort_by("id", false)?;
    let _ = df.filter(&[true, false, true])?;
    let _ = df.vstack(&df)?;
    let _ = df.with_row_index("i")?;

    // read a row back as a StructScalar record
    let record = asc.row(0)?;
    assert_eq!(record.child_named("name").unwrap().to_str(), "'a'::utf8");

    println!("{}", df.display(&DisplayOptions::default()));
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

### Schema-cast projection

`select_fields` projects **and casts** to an explicit target schema in one step: each
target [`Field`](../schema/field.md) takes the source column of the same name **cast to
its type** (or, if absent, a **filled** column — null when nullable, else the type
default), in the target order, dropping unlisted columns. The schema companion to
`select_columns` (which only reorders / projects), powered by the same `cast` struct
kernel.

=== "Python"

    ```python
    import yggdryl

    df = yggdryl.Serie.struct("df", [yggdryl.Serie("id", [1, 2])])  # id: int64
    target = [
        yggdryl.Field("id", yggdryl.DataType("int32"), True),       # narrow
        yggdryl.Field("score", yggdryl.DataType("float64"), True),  # missing -> filled null
    ]
    out = df.select_fields(target)
    assert out.column_names == ["id", "score"]
    assert out.child("score").value_at(0) is None
    ```

=== "Node"

    ```javascript
    const { Serie, Field, DataType } = require('yggdryl')

    const df = Serie.struct('df', [new Serie('id', [1, 2])])
    const out = df.selectFields([
      new Field('id', new DataType('int32'), true),
      new Field('score', new DataType('float64'), true),  // filled with null
    ])
    if (out.child('score').valueAt(0) !== null) throw new Error('fill')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, StructSerie, NestedSerie, Serie, SerieRef, DataType, Field, Scalar};
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let df = StructSerie::from_children("df", vec![id])?;
    let out = df.select_fields(vec![
        Field::new("id", DataType::int(64, true), true),     // widen
        Field::new("score", DataType::float(64), true),      // filled null
    ])?;
    assert_eq!(out.child_by_name("score").unwrap().value_at(0), Scalar::Null);
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

### Arrow interchange (RecordBatch / IPC / reader)

A frame round-trips through Arrow: `to_arrow_ipc()` writes an **Arrow IPC stream** (columns
as top-level fields) that any Arrow library reads back as a table, and
`from_arrow_ipc(name, bytes)` reads it in. In Rust there are also `to_record_batch` /
`from_record_batch`, chunked `to_record_batches(max_rows)` / `from_record_batches`, and a
streaming `to_record_batch_reader` / `from_record_batch_reader` (the shape Parquet readers
and scanners consume).

=== "Python"

    ```python
    import yggdryl
    import pyarrow as pa                                 # any Arrow library

    df = yggdryl.Serie.struct("df", [
        yggdryl.Serie("id", [1, 2, 3]),
        yggdryl.Serie("name", ["a", "b", "c"]),
    ])
    ipc = df.to_arrow_ipc()
    table = pa.ipc.open_stream(ipc).read_all()           # -> a pyarrow.Table
    assert table.column_names == ["id", "name"]
    back = yggdryl.Serie.from_arrow_ipc("df", ipc)        # round-trips
    assert back.to_dicts() == df.to_dicts()
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const df = Serie.struct('df', [new Serie('id', [1, 2, 3])])
    const ipc = df.toArrowIpc()                          // Buffer of an Arrow IPC stream
    const back = Serie.fromArrowIpc('df', ipc)
    if (back.shape[0] !== 3) throw new Error('roundtrip')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, StructSerie, Serie, SerieRef};
    use std::sync::Arc;

    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2), Some(3)]));
    let df = StructSerie::from_children("df", vec![id])?;

    // one RecordBatch, or chunked batches, or an IPC stream
    let batch = df.to_record_batch()?;
    assert_eq!(batch.num_rows(), 3);
    let chunks = df.to_record_batches(2)?;               // [2 rows, 1 row]
    let reader = df.to_record_batch_reader(2)?;           // a RecordBatchReader (scanner)
    let bytes = df.to_ipc_bytes()?;
    assert_eq!(StructSerie::from_ipc_bytes("df", &bytes)?.shape(), (3, 1));
    let _ = (chunks, reader);
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Update values: `set_at` & `push`

Arrow arrays are immutable, so the value mutators are **functional** — they return a *new*
column with one cell replaced (`set_at`) or a row appended (`push`), leaving the original
untouched. With `safe` (the default) the incoming [`Scalar`](../scalar/scalar.md) is
**cast to the column's type** first, so any value can be written; an out-of-bounds index
errors. The rebuild is uniform across every type — primitive, varchar, binary and
**nested** — via a single Arrow `concat`.

=== "Python"

    ```python
    import yggdryl

    s = yggdryl.Serie("n", [1, 2, 3])
    assert s.set_at(1, yggdryl.Scalar(20)).to_list() == [1, 20, 3]
    assert s.to_list() == [1, 2, 3]                      # original untouched
    assert s.set_at(0, yggdryl.Scalar.null("int64")).to_list() == [None, 2, 3]
    assert s.push(yggdryl.Scalar(4)).to_list() == [1, 2, 3, 4]
    ```

=== "Node"

    ```javascript
    const { Serie, Scalar } = require('yggdryl')

    const s = new Serie('n', [1, 2, 3])
    if (s.setAt(1, new Scalar(20)).toList().join() !== '1,20,3') throw new Error('set')
    if (s.toList().join() !== '1,2,3') throw new Error('functional')
    if (s.push(new Scalar(4)).toList().join() !== '1,2,3,4') throw new Error('push')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{Int32Serie, Scalar, Serie};
    use yggdryl_scalar::IntScalar;

    let s = Int32Serie::from_values("n", vec![Some(1), Some(2), Some(3)]);
    let updated = s.set_at(1, &IntScalar::new(20, 64, true), true)?;  // int64 cast to int32
    assert_eq!(updated.value_at(1), Scalar::Int(20));
    assert_eq!(s.value_at(1), Scalar::Int(2));                        // original untouched
    let grown = s.push(&IntScalar::new(4, 8, true), true)?;
    assert_eq!(grown.len(), 4);
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Display

`display` is the **single** render method — there is no separate `show`. A leaf column
renders **vertically** (one value per line); a struct [frame](#frame-dataframe) renders as
an **aligned table** (one column per child). Parameters: `max_rows`, `header`, `width`,
`null` and `index` (a leading row-index gutter).

=== "Python"

    ```python
    import yggdryl

    s = yggdryl.Serie("n", list(range(100)))
    assert "97 more rows" in s.display(max_rows=3)        # a column, vertical

    df = yggdryl.Serie.struct("df", [yggdryl.Serie("id", [1, 2])])
    assert "id: int64" in df.display()                   # a frame, table
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const s = new Serie('n', Array.from({ length: 100 }, (_, i) => i))
    if (!s.display(3).includes('97 more rows')) throw new Error('column')
    const df = Serie.struct('df', [new Serie('id', [1, 2])])
    if (!df.display().includes('id: int64')) throw new Error('table')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{DisplayOptions, Int32Serie, Serie, StructSerie, SerieRef};
    use std::sync::Arc;

    // a leaf column renders vertically
    let serie = Int32Serie::from_values("n", (0..100).map(Some));
    let text = serie.display(&DisplayOptions::default().with_max_rows(3));
    assert!(text.contains("n: int32"));        // header
    assert!(text.contains("97 more rows"));    // truncation marker

    // a struct frame renders as an aligned table (same method)
    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let df = StructSerie::from_children("df", vec![id])?;
    assert!(df.display(&DisplayOptions::default()).contains("id: int32"));
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Serialize

`to_bytes` writes the column as an **Arrow IPC stream** and `from_bytes` reads it back —
a **lossless** round-trip of the type, name, nulls and values, *including nested*
columns. This is the canonical bytes form: Python `pickle` / `copy` and Node
`toJSON` / `fromJSON` go through it, so any column round-trips faithfully.

=== "Python"

    ```python
    import copy, pickle, yggdryl

    s = yggdryl.Serie("n", [1, None, 3])
    assert yggdryl.Serie.from_bytes(s.to_bytes()).to_list() == [1, None, 3]
    assert pickle.loads(pickle.dumps(s)).to_list() == [1, None, 3]   # via IPC bytes
    rec = yggdryl.Serie.struct("rec", [yggdryl.Serie("a", [1, 2])])
    assert copy.copy(rec).select("a").to_list() == [1, 2]            # nested too
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const s = new Serie('n', [1, null, 3])
    const back = Serie.fromBytes(s.toBytes())
    if (back.toList().join() !== '1,,3') throw new Error('bytes')
    const json = JSON.stringify(s)                       // lossless via IPC hex
    if (Serie.fromJSON(JSON.parse(json)).get(2) !== 3) throw new Error('json')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{from_bytes, Int32Serie, Scalar, Serie};

    let serie = Int32Serie::from_values("n", vec![Some(1), None, Some(3)]);
    let bytes = serie.to_bytes()?;
    let back = from_bytes(&bytes)?;
    assert_eq!(back.name(), "n");
    assert_eq!(back.value_at(0), Scalar::Int(1));
    assert!(back.is_null(1));
    # Ok::<(), yggdryl_serie::SerieError>(())
    ```

## Coverage

The primitive category is complete: integers (`int8`…`int64`, `uint8`…`uint64`),
floats (`float16`/`32`/`64`), decimals (128/256), dates and intervals, boolean, UTF-8
strings (`Utf8` / `LargeUtf8`) and binary (`Binary` / `LargeBinary`); timestamps, times
and durations unify into `DatetimeSerie` / `TimeSerie` / `DurationSerie`. The **nested**
`StructSerie` (which holds **lazy children** — `from_children` stays lazy until
`materialize`) / `ListSerie` / `MapSerie` (recursive), the lazy `RangeSerie` /
`DateRangeSerie` / `DateTimeRangeSerie` / `TimeRangeSerie`, the `IndexSerie`,
`CategoricalSerie` (dictionary-encoded), the `TemporalSerie` / `NestedSerie` traits, the
`SliceSerie` graph, `cast` / `resize` / `display`, lossless Arrow-IPC `to_bytes` /
`from_bytes`, and a per-datatype default (`Scalar::default_for`) round it out. The column
API is **surfaced in the Python and Node bindings** (a single `Serie` class) and covered
by [benchmarks](../benchmarks.md#serie-the-columnar-layer). The **union** nested type, the
**view** backend, a **`ChunkedSerie`** mirroring Arrow's `ChunkedArray` and arithmetic
operations are the next increments.

## Next

- [DataType](../schema/datatype.md) — the logical type a serie carries
- [Field](../schema/field.md) — naming a column, building a schema
