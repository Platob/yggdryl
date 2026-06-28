# Serie

A `Serie` is a single named, **typed column** — the layer between the
[schema](../schema/datatype.md) type system and a future dataframe. It pairs a
[`Field`](../schema/field.md) (name + `DataType` + nullability + metadata) with an
Apache **Arrow** array holding the values, so a column carries both its logical type
and its physical storage. Columns can also be **lazy** (computed on demand) or
**children** (zero-copy slices that remember their parent).

!!! note "Available in all three languages"
    `yggdryl-serie` is the Arrow-backed columnar foundation. A **struct column is a
    [DataFrame](frame.md)** (its children are the columns), so the same `Serie` class is
    both the column *and* the table. The API is surfaced in **Python** and **Node** as a
    single `Serie` class — build from a list, read / update by index, slice / resize / cast,
    navigate nested children, run frame ops (select / filter / sort / stack / records),
    round-trip through bytes — as well as the Rust core, which also exposes the richer
    concrete-series internals (typed downcasts, the slice graph) directly.

This page covers the base column. See also: [Lazy, range & categorical](lazy.md) ·
[Nested (struct / list / map)](nested.md) · [Frame (DataFrame)](frame.md).

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
  types), `BooleanSerie`, `VarcharSerie<O>`, `BinarySerie<O>` and `NullSerie` (the all-null
  column), with named aliases (`Int32Serie`, `Float64Serie`, `Date32Serie`, …).
- The **temporal** series — `DatetimeSerie`, `TimeSerie` and `DurationSerie` (unified
  columns over any unit, presenting core `DateTime` / `Time` / `Duration`) and the
  `TemporalSerie` trait (`datetime_at` / `date_at` / `time_at`).
- The **[nested](nested.md)** series — `StructSerie`, `ListSerie<O>` and `MapSerie` (child
  columns built recursively) and the `NestedSerie` trait.
- The **[lazy](lazy.md)** (computed) series — the type-parameterised `RangeSerie<A>` (its
  `uint64` form, `UInt64RangeSerie`, doubles as the canonical row index), `DateRangeSerie`,
  `DateTimeRangeSerie`, `TimeRangeSerie`.
- **[`CategoricalSerie`](lazy.md#categorical-dictionary-encoded)** — a dictionary-encoded
  view for repeated values (distinct values + a per-row code), decoding back to a flat
  column on `materialize`.

## Build a column

In Rust, `from_array` derives the field from the Arrow type while `from_arrow` takes an
explicit `Field` (carrying name, nullability and metadata); both **redirect** the array to
the right concrete series and return a boxed `SerieRef`. In the bindings the `Serie`
constructor is the entry point, and every column reflects its `field` / `data_type` /
`nullable`.

=== "Python"

    ```python
    import yggdryl

    s = yggdryl.Serie("id", [1, None, 3])
    assert s.num_rows == 3 and s.null_count == 1
    assert str(s.data_type) == "int64"
    assert s.nullable is True
    assert s.field.name == "id"
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const s = new Serie('id', [1, null, 3])
    if (s.numRows !== 3 || s.nullCount !== 1) throw new Error('shape')
    if (s.dataType.toString() !== 'int64') throw new Error('type')
    if (s.nullable !== true) throw new Error('nullable')
    if (s.field.name !== 'id') throw new Error('field')
    ```

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
    ```

## Creating series — one line per type

In the bindings a single `Serie` builds any column from a list (the type is inferred, or
pass a `dtype`), with `range` / `index` / `struct` / `list` / `map` / `binary` factories
for the rest. In Rust each concrete series has a `from_values(name, values)` one-liner;
`from_array` is the universal fallback for *any* Arrow array.

### Type inference

The element type is inferred from the first non-null value, adapting to each language's
number model:

| value | Python | Node (JS has one number type) |
| --- | --- | --- |
| whole number | `int64` | `int64` **only if every value is integral**, else `float64` |
| fractional number | `float64` | `float64` |
| `True` / `False` | `bool` | `bool` |
| string | `utf8` | `utf8` |
| bytes | `binary` | use the `Serie.binary` factory |
| list / array | `list<…>` (the [nested](nested.md) factory) | `list<…>` |
| dict / object | `map<…>` (the [nested](nested.md) factory) | `map<…>` |

A **nested** value (a Python list/dict, a JS array/object) makes the constructor build a
list / map column, **recursively** — the element builder is the same constructor, so a
list of dicts is `list<map>`, a list of lists is `list<list>`, and so on. Nulls are skipped
during inference; an **empty or all-null** list cannot be inferred, so pass an explicit
`dtype` (a `DataType` or a type string like `"int8"`). Passing `dtype` always **casts** the
leaf type. Rust does not infer — pick the concrete `*Serie` (or `from_array` for an
existing Arrow array).

=== "Python"

    ```python
    import yggdryl

    lists = yggdryl.Serie("a", [[1, 2], [], None, [3]])  # list<int64> (a list value)
    maps = yggdryl.Serie("m", [{"x": 1}, {"y": 2}])       # map<utf8, int64> (a dict value)
    nested = yggdryl.Serie("ld", [[{"a": 1}], [{"b": 2}]]) # list<map<utf8, int64>>
    assert str(lists.data_type) == "list[item: int64]"
    assert maps.value_at(0) == "{x=1}"
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const lists = new Serie('a', [[1, 2], [], null, [3]])  // list<int64> (an array value)
    const maps = new Serie('m', [{ x: 1 }, { y: 2 }])      // map<utf8, int64> (an object value)
    const nested = new Serie('ld', [[{ a: 1 }], [{ b: 2 }]]) // list<map<utf8, int64>>
    if (lists.dataType.toString() !== 'list[item: int64]') throw new Error('list')
    if (maps.valueAt(0) !== '{x=1}') throw new Error('map')
    ```

=== "Rust"

    ```rust
    // Rust has no value inference — pick the concrete series (see Nested for the builders).
    use yggdryl_serie::{Int32Serie, ListSerie, Serie, SerieRef, Scalar};
    use std::sync::Arc;

    let flat: SerieRef = Arc::new(Int32Serie::from_values("item", vec![Some(1), Some(2), Some(3)]));
    let lists = ListSerie::<i32>::from_values("a", flat, &[Some(2), Some(0), None, Some(1)])?;
    assert_eq!(lists.value_at(0), Scalar::Other("[1, 2]".into()));
    ```

=== "Python"

    ```python
    import yggdryl

    ints = yggdryl.Serie("i", [1, None, 3])                 # int64 (whole numbers)
    floats = yggdryl.Serie("f", [1.5, 2.5])                 # float64
    flags = yggdryl.Serie("b", [True, False])               # bool
    text = yggdryl.Serie("s", ["a", "b"])                   # utf8
    small = yggdryl.Serie("i8", [1, 2, 3], dtype="int8")    # explicit dtype (cast)
    empty = yggdryl.Serie("e", [], dtype="float64")         # empty needs a dtype

    rng = yggdryl.Serie.range(100)                          # lazy 0..100 (see Lazy)
    idx = yggdryl.Serie.index(100)                          # lazy row index
    rec = yggdryl.Serie.struct("rec", [                     # nested (see Nested)
        yggdryl.Serie("id", [1, 2]),
        yggdryl.Serie("name", ["a", "b"]),
    ])
    cat = yggdryl.Serie("c", ["a", "b", "a"]).categorical() # dictionary-encoded (see Lazy)
    assert rec.children()[0].name == "id"
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const ints = new Serie('i', [1, null, 3])               // int64 (all integral)
    const floats = new Serie('f', [1.5, 2.5])               // float64
    const mixed = new Serie('m', [1, 2.5])                  // float64 (one fractional value)
    const flags = new Serie('b', [true, false])             // bool
    const text = new Serie('s', ['a', 'b'])                 // utf8
    const small = new Serie('i8', [1, 2, 3], 'int8')        // explicit dtype (cast)

    const rng = Serie.range(100)                            // lazy 0..100 (see Lazy)
    const idx = Serie.index(100)                            // lazy row index
    const rec = Serie.struct('rec', [                       // nested (see Nested)
      new Serie('id', [1, 2]),
      new Serie('name', ['a', 'b']),
    ])
    const cat = new Serie('c', ['a', 'b', 'a']).categorical() // dictionary-encoded (see Lazy)
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{
        Int32Serie, Float64Serie, BooleanSerie, VarcharSerie, BinarySerie,
        DatetimeSerie, TimeSerie, DurationSerie, DateRangeSerie, UInt64RangeSerie,
        StructSerie, CategoricalSerie, NestedSerie, Serie, SerieRef, TypedSerie,
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

    // lazy ranges + the row index (computed, not stored)
    let r = UInt64RangeSerie::uint64("r", 0, 1, 100);     // 0..100 (uint64)
    let days = DateRangeSerie::from_dates("d", Date::from_ymd(2024, 1, 1).unwrap(), 1, 7);
    let idx = UInt64RangeSerie::indices(100);          // the canonical row index

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
    ```

The lazy ranges and the row index live on the [Lazy & range](lazy.md) page; struct / list /
map building is on the [Nested](nested.md) page.

## Values: by index and by range

`value_at` reads a single cell as a type-erased `Scalar` (`Null` for a null or
out-of-bounds cell); `slice` / `slice_range` return a **zero-copy** sub-column. For
typed access, downcast to the concrete series and use `TypedSerie<T>`.

=== "Python"

    ```python
    import yggdryl

    s = yggdryl.Serie("n", [5, None, 7])

    # by index -> a native value (None for a null or out-of-bounds cell)
    assert s.value_at(0) == 5
    assert s.value_at(1) is None                        # null cell
    assert s.value_at(9) is None                        # out of bounds
    assert s[2] == 7                                     # subscript (supports negatives)

    # by range -> a zero-copy slice
    assert len(s.slice(1, 2)) == 2

    # null / presence checks
    assert s.is_null(1)
    assert s.is_valid(0)
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const s = new Serie('n', [5, null, 7])

    // by index -> a native value (null for a null or out-of-bounds cell)
    if (s.valueAt(0) !== 5) throw new Error('value')
    if (s.valueAt(1) !== null) throw new Error('null')  // null cell
    if (s.get(2) !== 7) throw new Error('get')          // supports negative indices

    // by range -> a zero-copy slice
    if (s.slice(1, 2).numRows !== 2) throw new Error('slice')

    // null / presence checks
    if (!s.isNull(1)) throw new Error('isNull')
    if (!s.isValid(0)) throw new Error('isValid')
    ```

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
    ```

## Defaults & resize

Every datatype has a default value (`Scalar::default_for`): `false`, `0`, `0.0`, the
empty string, empty bytes, a struct of defaults. `resize(new_len)` slices when
shrinking and extends when growing — with **nulls** if the column is nullable, otherwise
the type **default** (so a non-nullable column never gains a null).

=== "Python"

    ```python
    import yggdryl

    s = yggdryl.Serie("n", [1, 2])
    assert s.resize(4).value_at(3) is None              # grow: a nullable column gains nulls
    assert len(s.resize(1)) == 1                         # shrink: a zero-copy slice
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const s = new Serie('n', [1, 2])
    if (s.resize(4).valueAt(3) !== null) throw new Error('grow')  // nullable -> nulls
    if (s.resize(1).numRows !== 1) throw new Error('shrink')      // a slice
    ```

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
    ```

## The slice graph: children & parents

`slice` / `slice_range` build a **zero-copy** view over the same buffers; `materialize()`
realises a column into an independent, in-memory one. In the Rust core a slice additionally
remembers the serie it came from via `parent()` — a navigable graph (the `child` /
`child_range` free functions, and `materialize` **detaches** the view). The bindings expose
the zero-copy `slice` / `head` and `materialize`; the `parent()` back-link is a Rust-core
detail.

=== "Python"

    ```python
    import yggdryl

    parent = yggdryl.Serie("n", [10, 20, 30, 40])
    view = parent.slice(1, 2)                            # rows 1..3, zero-copy
    assert view.to_list() == [20, 30]
    assert parent.to_list() == [10, 20, 30, 40]          # original untouched
    independent = view.materialize()                     # an in-memory copy
    assert independent.is_materialized
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const parent = new Serie('n', [10, 20, 30, 40])
    const view = parent.slice(1, 2)                      // rows 1..3, zero-copy
    if (view.toList().join() !== '20,30') throw new Error('slice')
    const independent = view.materialize()               // an in-memory copy
    if (!independent.isMaterialized) throw new Error('materialized')
    ```

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
    ```

## Temporal series

`DatetimeSerie` is the **unified timestamp column**: it backs any unit (second …
nanosecond) and an optional timezone, exposing values as the core `DateTime`. `TimeSerie`
and `DurationSerie` are its time-of-day and elapsed-time counterparts — each unifies every
unit and presents core `Time` / `Duration` (so there are no per-unit aliases). Every
timestamp / time / duration array dispatches to these. All temporal columns (including the
date/time/datetime ranges) implement `TemporalSerie` — a uniform `datetime_at` with derived
`date_at` / `time_at` (`DurationSerie` is a span, so it is not `TemporalSerie`). In the
bindings a temporal column is built by casting to a temporal `dtype`; `value_at` returns the
physical (the epoch count), while the Rust core presents the typed `DateTime` / `Time` /
`Duration`.

=== "Python"

    ```python
    import yggdryl

    # cast an integer column to a temporal dtype
    ts = yggdryl.Serie("ts", [0, 86400], dtype="timestamp[s]")
    assert str(ts.data_type) == "timestamp[s]"
    assert ts.value_at(1) == 86400                       # physical (epoch seconds)
    d = yggdryl.Serie("d", [0, 1, 2], dtype="date32")
    assert str(d.data_type) == "date32"
    assert d.value_at(2) == 2                             # days since the epoch
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const ts = new Serie('ts', [0, 86400], 'timestamp[s]')
    if (ts.dataType.toString() !== 'timestamp[s]') throw new Error('type')
    if (ts.valueAt(1) !== 86400) throw new Error('physical')  // epoch seconds
    const d = new Serie('d', [0, 1, 2], 'date32')
    if (d.valueAt(2) !== 2) throw new Error('days')           // days since the epoch
    ```

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
    ```

## Cast

`cast(dtype)` converts a column's values (Arrow's cast kernel — including lossy /
narrowing casts, which yield null on overflow). A **struct → struct** cast matches
children by name, casts each, **fills missing** target columns (null if nullable, else
the type default) and drops extras. `dtype` is a `DataType` **or a type string** — the
bindings accept either, and Rust adds `cast_str(&str)` next to the canonical
`cast(&DataType)`; both spellings parse through `DataType::from_str` and run the one
`cast` implementation.

`cast` **prechecks the target type**: if it equals the column's current type, or is the
wildcard `any`, the cast is **skipped** — the column's own array is re-wrapped unchanged,
with no Arrow-kernel conversion (its type and values are preserved). The `null` type is
likewise fast cast: casting **to or from `null`** builds an all-null column of the target
type directly — the natural target for an all-null column, and a `null` column casts back
to any type as an all-null fill.

=== "Python"

    ```python
    import yggdryl

    ints = yggdryl.Serie("n", [1, 2, 3])
    assert ints.cast("float64")[0] == 1.0
    big = yggdryl.Serie("n", [1000, 5]).cast("int8")    # narrowing
    assert big[0] is None                               # 1000 overflows int8

    # fast casts: to `any` (no-op) and to / from `null`
    assert str(ints.cast("any").data_type) == "int64"   # any keeps the concrete type
    nulled = ints.cast("null")                          # -> an all-null column
    assert str(nulled.data_type) == "null" and nulled.null_count == 3
    assert nulled.cast("utf8").value_at(0) is None       # null casts back as all-null
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const ints = new Serie('n', [1, 2, 3])
    if (ints.cast('float64').get(0) !== 1) throw new Error('cast')
    const big = new Serie('n', [1000, 5]).cast('int8')  // narrowing
    if (big.get(0) !== null) throw new Error('overflow') // 1000 overflows int8

    // fast casts: to `any` (no-op) and to / from `null`
    if (ints.cast('any').dataType.toString() !== 'int64') throw new Error('any')
    const nulled = ints.cast('null')                    // -> an all-null column
    if (nulled.dataType.toString() !== 'null' || nulled.nullCount !== 3) throw new Error('null')
    if (nulled.cast('utf8').valueAt(0) !== null) throw new Error('from-null')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{from_array, Int32Serie, NullSerie, StructSerie, DataType, Field, NestedSerie, Serie, Scalar, SerieRef};
    use yggdryl_serie::arrow_array::{ArrayRef, Int32Array};
    use std::sync::Arc;

    // primitive cast (lossy narrowing yields null on overflow); by DataType or by string
    let ints = from_array("n", Arc::new(Int32Array::from(vec![1, 2, 3])) as ArrayRef)?;
    assert_eq!(ints.cast(&DataType::float(64))?.value_at(0), Scalar::Float(1.0));
    assert_eq!(ints.cast_str("float64")?.value_at(0), Scalar::Float(1.0)); // string convenience

    // fast casts: to `Any` (no-op) and to / from `Null`
    assert_eq!(ints.cast(&DataType::Any)?.data_type(), &DataType::int(32, true));
    let nulled = ints.cast(&DataType::Null)?;            // an all-null NullSerie
    assert_eq!(nulled.null_count(), 3);
    assert!(nulled.as_any().downcast_ref::<NullSerie>().is_some());
    assert_eq!(nulled.cast(&DataType::varchar())?.value_at(0), Scalar::Null);

    // struct cast with a missing column filled
    let id: SerieRef = Arc::new(Int32Serie::from_values("id", vec![Some(1), Some(2)]));
    let rec: SerieRef = Arc::new(StructSerie::from_children("rec", vec![id])?);
    let target = DataType::struct_(vec![
        Field::new("id", DataType::int(64, true), true),    // widened
        Field::new("extra", DataType::varchar(), true),     // missing -> filled null
    ]);
    let casted = rec.cast(&target)?;
    assert_eq!(casted.as_nested().unwrap().child_by_name("extra").unwrap().value_at(0), Scalar::Null);
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
    ```

## Display

`display` is the **single** render method — there is no separate `show`. A leaf column
renders **vertically** (one value per line); a struct [frame](frame.md) renders as an
**aligned table** (one column per child). Parameters: `max_rows`, `header`, `width`,
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
    ```

## Coverage

The primitive category is complete: integers (`int8`…`int64`, `uint8`…`uint64`),
floats (`float16`/`32`/`64`), decimals (128/256), dates and intervals, boolean, UTF-8
strings (`Utf8` / `LargeUtf8`), binary (`Binary` / `LargeBinary`) and the all-null `null`
(`NullSerie`); timestamps, times
and durations unify into `DatetimeSerie` / `TimeSerie` / `DurationSerie`. The **nested**
`StructSerie` (which holds **lazy children** — `from_children` stays lazy until
`materialize`) / `ListSerie` / `MapSerie` (recursive), the lazy type-parameterised
`RangeSerie<A>` (its `uint64` form doubling as the row index) / `DateRangeSerie` /
`DateTimeRangeSerie` / `TimeRangeSerie`,
`CategoricalSerie` (dictionary-encoded), the `TemporalSerie` / `NestedSerie` traits, the
`SliceSerie` graph, `cast` / `resize` / `display`, lossless Arrow-IPC `to_bytes` /
`from_bytes`, and a per-datatype default (`Scalar::default_for`) round it out. The column
API is **surfaced in the Python and Node bindings** (a single `Serie` class) and covered
by [benchmarks](../benchmarks.md#serie-the-columnar-layer). The **union** nested type, the
**view** backend, a **`ChunkedSerie`** mirroring Arrow's `ChunkedArray` and arithmetic
operations are the next increments.

## Next

- [Lazy, range & categorical](lazy.md) — computed columns and the `uint64` row index
- [Nested (struct / list / map)](nested.md) — columns of columns and the build factories
- [Frame (DataFrame)](frame.md) — a struct column *is* a table
- [DataType](../schema/datatype.md) — the logical type a serie carries
- [Field](../schema/field.md) — naming a column, building a schema
