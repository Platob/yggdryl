# Lazy, range & categorical series

Some columns are **not** fully resident in memory. A **lazy** column stores only a compact
description and computes each value on demand (`is_materialized()` is `false`) until
`materialize()` realises a real Arrow array. This page covers the lazy ranges, the
datatype-generic range that doubles as the canonical **row index**, and the
dictionary-encoded `CategoricalSerie`.

See also: [Serie (the typed column)](serie.md) · [Nested](nested.md) · [Frame](frame.md).

## Lazy (computed) ranges

A range stores a `start`, a `step` and a length, and computes `start + step*i` on demand.

- `RangeSerie<A>` — a **type-parameterised** arithmetic range over an Arrow primitive type
  `A` (like `PrimitiveSerie<A>`). It stores `start` / `step` as the **native physical** value
  (`u64`, `i64`, a timestamp's `i64`, …) and computes each value with **native arithmetic**,
  building a `PrimitiveArray<A>` directly — so an integer or timestamp range is as cheap as a
  typed array read, with no boxing. A `uint64` one (`UInt64RangeSerie`) is the row index
  (below); a date range is `RangeSerie<Date32Type>`, a timestamp range
  `RangeSerie<TimestampSecondType>`, and so on. In the bindings `Serie.range` / `Serie.index`
  build the `uint64` form.
- `DateRangeSerie` — a day-resolution calendar-date range (`Date32`).
- `DateTimeRangeSerie` — a nanosecond timestamp range.
- `TimeRangeSerie` — a time-of-day range (wraps within the day).

The three temporal ranges implement `TemporalSerie` (see [temporal series](serie.md#temporal-series)).
The bindings expose the arithmetic `Serie.range`; the calendar/time ranges are Rust-only.

=== "Python"

    ```python
    import yggdryl

    r = yggdryl.Serie.range(4, start=100, step=5)        # 100, 105, 110, 115 (lazy)
    assert not r.is_materialized
    assert r[2] == 110
    realized = r.materialize()                           # -> a real uint64 column
    assert realized.is_materialized
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const r = Serie.range(4, 100, 5)                     // 100, 105, 110, 115 (lazy)
    if (r.isMaterialized) throw new Error('lazy')
    if (r.get(2) !== 110) throw new Error('value')
    const realized = r.materialize()                     // -> a real uint64 column
    if (!realized.isMaterialized) throw new Error('materialized')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{RangeSerie, UInt64RangeSerie, DateRangeSerie, Serie, Scalar};
    use yggdryl_serie::arrow_array::types::Float64Type;
    use yggdryl_core::Date;

    let r = UInt64RangeSerie::uint64("r", 100, 5, 4);  // 100, 105, 110, 115 (lazy uint64)
    assert!(!r.is_materialized());
    assert_eq!(r.at(2), Some(110));
    assert_eq!(r.value_at(3), Scalar::Int(115));
    let realized = r.materialize();                    // -> a real uint64 column
    assert!(realized.is_materialized());

    // type-parameterised: a float64 range, native arithmetic
    let f = RangeSerie::<Float64Type>::new("f", 1.0, 0.5, 4);
    assert_eq!(f.value_at(3), Scalar::Float(2.5));

    let dates = DateRangeSerie::from_dates("d", Date::from_ymd(2024, 1, 30).unwrap(), 1, 3);
    assert_eq!(dates.date_at(2), Some(Date::from_ymd(2024, 2, 1).unwrap()));
    ```

## Range & row index

The `uint64` `UInt64RangeSerie` is the canonical **row index** — the `0, 1, …, len-1`
labels that address a frame's rows, the implicit index a frame carries when no explicit
one is set.
Because the values are a known arithmetic progression, the label ↔ position lookups
(`at` / `position` / `contains`) are **O(1)**, even after a slice (whose labels start at
the slice offset). `Serie.index(len)` is the `0..len` index; `Serie.range(...)` is the
general `start + i*step` range; `is_range` reports whether a column is the canonical
`0..len` index (`start == 0`, `step == 1`).

=== "Python"

    ```python
    import yggdryl

    index = yggdryl.Serie.index(4)                       # lazy [0, 1, 2, 3] (uint64)
    assert index.is_range
    assert not index.is_materialized
    assert index.at(2) == 2                              # label at row 2
    assert index.position(3) == 3                        # row of label 3
    assert not index.contains(4)

    stepped = yggdryl.Serie.range(4, start=100, step=5)  # 100, 105, 110, 115
    assert not stepped.is_range                          # start != 0
    assert stepped.position(110) == 2                    # O(1) inverse lookup
    assert yggdryl.Serie("n", [1, 2]).is_range is False  # a plain column is not a range
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const index = Serie.index(4)                         // lazy [0, 1, 2, 3] (uint64)
    if (!index.isRange) throw new Error('range')
    if (index.isMaterialized) throw new Error('lazy')
    if (index.at(2) !== 2) throw new Error('label')      // label at row 2
    if (index.position(3) !== 3) throw new Error('pos')  // row of label 3
    if (index.contains(4)) throw new Error('absent')

    const stepped = Serie.range(4, 100, 5)               // 100, 105, 110, 115
    if (stepped.isRange) throw new Error('not canonical') // start != 0
    if (stepped.position(110) !== 2) throw new Error('inverse')
    if (new Serie('n', [1, 2]).isRange) throw new Error('plain')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{UInt64RangeSerie, Serie};

    let index = UInt64RangeSerie::indices(4);   // lazy [0, 1, 2, 3] (uint64)
    assert!(index.is_range());
    assert!(!index.is_materialized());
    assert_eq!(index.at(2), Some(2));                 // label at row 2
    assert_eq!(index.position(3), Some(3));           // row of label 3
    assert!(!index.contains(4));

    let stepped = UInt64RangeSerie::uint64("r", 100, 5, 4); // 100, 105, 110, 115
    assert!(!stepped.is_range());                     // start != 0
    assert_eq!(stepped.position(110), Some(2));       // O(1) inverse lookup
    ```

Slicing a range stays a lazy `RangeSerie<A>` whose labels start at the slice offset, so
`at` / `position` keep working, but `is_range` becomes `false` (the labels no longer start
at `0`). Materialising a range yields a plain in-memory `uint64` column.

## Casting a range preserves its original type

[`cast`](serie.md#cast) on a range is special: it **keeps the original `start` / `end` /
`step`** and only re-types what the column *exposes*. The result is a **still-lazy** range
that computes in the original type and presents `value_at` / `array` / `data_type` as the
cast output — so the original numbers survive while the column reads as the new type.

=== "Python"

    ```python
    import yggdryl

    floats = yggdryl.Serie.index(4).cast("float64")   # cast a uint64 index
    assert not floats.is_materialized                 # still a lazy computed range
    assert str(floats.data_type) == "float64"
    assert floats.to_list() == [0.0, 1.0, 2.0, 3.0]   # exposed as floats
    assert floats.materialize().to_list() == [0.0, 1.0, 2.0, 3.0]
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const floats = Serie.index(4).cast('float64')     // cast a uint64 index
    if (floats.isMaterialized) throw new Error('lazy') // still a lazy computed range
    if (floats.dataType.toString() !== 'float64') throw new Error('type')
    if (floats.toList().join() !== '0,1,2,3') throw new Error('values')
    ```

=== "Rust"

    ```rust
    use yggdryl_serie::{UInt64RangeSerie, DataType, Serie, Scalar};

    let floats = UInt64RangeSerie::indices(4).cast(&DataType::float(64)).unwrap();
    assert_eq!(floats.data_type(), &DataType::float(64));   // exposes float
    assert!(!floats.is_materialized());                     // still lazy
    assert_eq!(floats.value_at(2), Scalar::Float(2.0));

    // the underlying range keeps its original uint64 start / step
    let range = floats.as_any().downcast_ref::<UInt64RangeSerie>().unwrap();
    assert!(range.is_cast());
    assert_eq!(range.start(), 0);                           // native u64 progression
    assert_eq!(range.original_type(), DataType::int(64, false));
    ```

## Categorical (dictionary-encoded)

`CategoricalSerie` is a **dictionary-encoded** view for *repeated values*: it stores the
distinct values once plus a compact per-row code, so a low-cardinality column is held
compactly. It is lazy (`is_materialized()` is `false`); `materialize()` decodes it back
into a flat column.

=== "Python"

    ```python
    import yggdryl

    cat = yggdryl.Serie("c", ["a", "b", "a"]).categorical()
    assert cat.category_count == 2                       # "a", "b" stored once
    assert cat.code_at(0) == cat.code_at(2)              # repeated "a" shares a code
    assert cat[1] == "b"
    assert not cat.is_materialized

    flat = cat.materialize()                             # decode -> a real varchar column
    assert flat.is_materialized
    ```

=== "Node"

    ```javascript
    const { Serie } = require('yggdryl')

    const cat = new Serie('c', ['a', 'b', 'a']).categorical()
    if (cat.categoryCount !== 2) throw new Error('count')        // "a", "b" stored once
    if (cat.codeAt(0) !== cat.codeAt(2)) throw new Error('code') // repeated "a" shares a code
    if (cat.get(1) !== 'b') throw new Error('value')
    if (cat.isMaterialized) throw new Error('lazy')

    const flat = cat.materialize()                              // decode -> a real column
    if (!flat.isMaterialized) throw new Error('decoded')
    ```

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

## Next

- [Serie (the typed column)](serie.md) — the base column, values, cast, display
- [Nested (struct / list / map)](nested.md) — columns of columns
- [Frame (DataFrame)](frame.md) — a struct column *is* a table
