# Interval

A calendar span in one of Arrow's three layouts: months, days and nanoseconds, none of them a count of another.

## Contract

| | |
| --- | --- |
| Owned | `IntervalType` with its single `Interval(layout)` leaf, `DataType::Interval(IntervalType)`, the `IntervalField` marker, and the `Interval` value holding all three components |
| Validated | once, at construction: `DataType::interval(unit)` refuses anything that is not one of the three layouts, and `Interval::new` refuses a component the layout has nowhere to put |
| Lazy | nothing; the leaf is `Copy` and the value is three components and a layout |
| Cached | the [field](../field.md)'s Arrow projection; the datatype caches nothing |
| Refused | a clock resolution or a day as the layout, a component the layout lacks, arithmetic with any other temporal, a restated count in another unit, and a rendering as classic text |

## DataType

One leaf, whose one parameter is the layout - a `TimeUnit` that is an interval
member rather than a resolution. `DataType::interval(unit)` validates it, where
the bare variant used to accept anything.

| leaf | `DataTypeId` | `as_u8()` | holds | unit | Arrow |
| --- | --- | --- | --- | --- | --- |
| `interval(layout)` | `Interval` | `0x38` | months, days and nanoseconds; a layout's missing components at zero | `year_month`, `day_time`, `month_day_nano` | `Interval(layout)` |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `interval(month_day_nano)` | the default layout | `interval` |
| `interval(day_time)` | days and milliseconds | SQL's bare `interval day` |
| `interval(year_month)` | whole months | SQL's bare `interval year`, `interval(years)` |

In SQL, bare `INTERVAL DAY` names the day-time layout, while the parenthesized
`interval(day)` still names the scalar day unit and is refused rather than
reinterpreted.

=== "Rust"

    ```rust
    use yggdryl::IntervalType;
    use yggdryl::{DataType, DataTypeId, DataTypeKind, TimeUnit};

    // One leaf, three layouts, and the layout is the parameter.
    let span = DataType::interval(TimeUnit::DayTime)?;
    assert_eq!(span, DataType::Interval(IntervalType::Interval(TimeUnit::DayTime)));
    assert_eq!(span.interval_type(), Some(IntervalType::Interval(TimeUnit::DayTime)));
    assert_eq!(span.to_string(), "interval(day_time)");
    assert_eq!(IntervalType::ALL, [IntervalType::Interval(TimeUnit::MonthDayNano)]);

    // The leaf answers its own family and identifier; the id names no layout.
    let leaf = IntervalType::Interval(TimeUnit::YearMonth);
    assert_eq!(leaf.id(), DataTypeId::Interval);
    assert_eq!(leaf.as_str(), "interval");
    assert_eq!(leaf.unit(), TimeUnit::YearMonth);
    assert_eq!(leaf.family(), "interval");
    assert_eq!(IntervalType::from_id(DataTypeId::Interval, TimeUnit::DayTime), Some(IntervalType::Interval(TimeUnit::DayTime)));
    assert_eq!(DataTypeId::Interval.as_u8(), 0x38);
    assert_eq!(span.kind(), DataTypeKind::Temporal);

    // The grammar: bare, parenthesized, and SQL's bare word.
    assert_eq!(DataType::from_str("interval")?, DataType::interval(TimeUnit::MonthDayNano)?);
    assert_eq!(DataType::from_str("interval day")?, span);
    assert_eq!(DataType::from_str("interval year")?, DataType::interval(TimeUnit::YearMonth)?);
    assert_eq!(DataType::from_str("interval(year_month)")?, DataType::interval(TimeUnit::YearMonth)?);

    // A resolution is no layout, wherever the rule is checked.
    assert!(DataType::from_str("interval(day)").is_err());
    assert!(DataType::interval(TimeUnit::Second).is_err());
    assert!(IntervalType::Interval(TimeUnit::Second).validate().is_err());
    assert!(DataType::Interval(IntervalType::Interval(TimeUnit::Nanosecond)).validate().is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType

    # One leaf, three layouts; bare `interval` is the widest one.
    assert DataType("interval") == DataType("interval(month_day_nano)")
    assert DataType("interval day") == DataType("interval(day_time)")
    assert DataType("interval year") == DataType("interval(year_month)")
    assert DataType("interval(day_time)").id == "interval"
    assert DataType("interval").kind == "temporal"

    # Arrow's widest interval storage, and the import back.
    assert DataType("interval").into_arrow() == pa.month_day_nano_interval()
    assert DataType.from_arrow(pa.month_day_nano_interval()) == DataType("interval(month_day_nano)")

    # A resolution is no layout, and the parenthesized day is the scalar unit.
    with pytest.raises(ValueError):
        DataType("interval(day)")
    with pytest.raises(ValueError):
        DataType("interval(s)")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // One leaf, three layouts; bare `interval` is the widest one.
    assert.ok(DataType.from('interval').equals(DataType.from('interval(month_day_nano)')))
    assert.ok(DataType.from('interval day').equals(DataType.from('interval(day_time)')))
    assert.ok(DataType.from('interval year').equals(DataType.from('interval(year_month)')))
    assert.equal(new DataType('interval(day_time)').id, 'interval')
    assert.equal(new DataType('interval').kind, 'temporal')

    // A resolution is no layout, and the parenthesized day is the scalar unit.
    assert.throws(() => new DataType('interval(day)'))
    assert.throws(() => new DataType('interval(s)'))
    ```

## Field

`IntervalField` is the typed marker, carrying `IntervalType` itself so the
layout is read off the payload. The bindings have one factory, the layout
defaulting to `month_day_nano`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, TimeUnit};
    use yggdryl::{IntervalField, IntervalType};

    let tenor = IntervalField::try_new(
        "tenor",
        DataType::interval(TimeUnit::MonthDayNano)?,
        false,
    )?;
    assert_eq!(*tenor.typed_dtype_ref(), IntervalType::Interval(TimeUnit::MonthDayNano));
    assert_eq!(tenor.typed_dtype_ref().unit(), TimeUnit::MonthDayNano);
    assert_eq!(tenor.id(), DataTypeId::Interval);

    // Another family is refused by the marker.
    assert!(IntervalField::try_new("tenor", DataType::duration64(TimeUnit::Millisecond)?, false).is_err());

    // The payload constructor takes the leaf directly.
    let months = IntervalField::new("months", IntervalType::Interval(TimeUnit::YearMonth), true);
    assert_eq!(months.dtype(), &DataType::interval(TimeUnit::YearMonth)?);
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType, types

    assert types.interval("tenor", "month_day_nano").dtype == DataType("interval(month_day_nano)")
    assert types.interval("months", "year_month").dtype == DataType("interval(year_month)")

    # The layout defaults to the widest one.
    assert types.interval("tenor").dtype == DataType("interval")
    assert types.interval("tenor", nullable=False).nullable is False

    with pytest.raises(ValueError, match="interval layout"):
        types.interval("window", "us")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    assert.equal(fields.interval('tenor', 'month_day_nano').dtype.toString(), 'interval(month_day_nano)')
    assert.equal(fields.interval('months', 'year_month').dtype.toString(), 'interval(year_month)')

    // The layout defaults to the widest one.
    assert.ok(fields.interval('tenor').dtype.equals(DataType.from('interval')))
    assert.equal(fields.interval('tenor', 'day_time', { nullable: false }).nullable, false)
    assert.throws(() => fields.interval('window', 'us'))
    ```

## Scalar

`Interval` holds every component of every layout - months, days and
nanoseconds - with a layout's missing components at zero, so no layout loses
one. `Interval::new` keeps a value from claiming a layout it does not fit, and
`Scalar::interval(months, days, nanoseconds, unit)` is the same rule through
the value door.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Interval, Scalar, TimeUnit};

    let span = Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano)?;
    let held = Interval::new(1, 2, 3, TimeUnit::MonthDayNano)?;
    assert_eq!(span, Scalar::Interval(held));
    assert_eq!(held.months(), 1);
    assert_eq!(held.days(), 2);
    assert_eq!(held.nanoseconds(), 3);
    assert_eq!(held.unit(), TimeUnit::MonthDayNano);

    // The shared readers answer the nanosecond component and the layout.
    assert_eq!(span.temporal_count(), Some(3));
    assert_eq!(span.temporal_unit(), Some(TimeUnit::MonthDayNano));
    assert_eq!(span.temporal_dtype(), Some(DataType::interval(TimeUnit::MonthDayNano)?));
    // No count restates across units: a month is no count of days.
    assert_eq!(span.temporal_count_at(TimeUnit::Nanosecond), None);

    // A layout refuses a component it has nowhere to put.
    assert!(Interval::new(1, 0, 0, TimeUnit::DayTime).is_err());
    assert!(Scalar::interval(1, 2, 0, TimeUnit::YearMonth).is_err());
    assert!(Scalar::interval(0, 0, 0, TimeUnit::Second).is_err());
    ```

=== "Python"

    ```python
    from yggdryl import Field, Scalar
    from yggdryl.text import json

    def typed(document: str, dtype: str) -> Scalar:
        return json.loads(document, field=Field("span", dtype, False), cls=Scalar)

    # Each layout keeps its own flat shape: one number, a pair, or a triple.
    assert typed("12", "interval(year_month)").as_py() == 12
    assert typed("[2,3]", "interval(day_time)").as_py() == [2, 3]
    assert typed("[1,2,3]", "interval(month_day_nano)").as_py() == [1, 2, 3]
    assert typed("12", "interval(year_month)").kind == "interval"
    assert typed("12", "interval(year_month)").family == "temporal"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, json } = require('yggdryl')

    const typed = (document, dtype) =>
      json.loads(document, { field: new Field('span', dtype, false), scalar: true })

    // Each layout keeps its own flat shape: one number, a pair, or a triple.
    assert.equal(typed('12', 'interval(year_month)').asJs(), 12)
    assert.deepEqual(typed('[2,3]', 'interval(day_time)').asJs(), [2, 3])
    assert.deepEqual(typed('[1,2,3]', 'interval(month_day_nano)').asJs(), [1, 2, 3])
    assert.equal(typed('12', 'interval(year_month)').kind, 'interval')
    ```

## Arrow storage

| datatype | Arrow | imports back as |
| --- | --- | --- |
| `interval(year_month)` | `Interval(YearMonth)` | `interval(year_month)` |
| `interval(day_time)` | `Interval(DayTime)` | `interval(day_time)` |
| `interval(month_day_nano)` | `Interval(MonthDayNano)` | `interval(month_day_nano)` |

All three round-trip, so an interval column crosses a boundary as the layout it
declared. A leaf built by hand around a resolution fails the projection as it
fails `validate`.

```rust
use arrow_schema::{DataType as ArrowDataType, IntervalUnit};
use yggdryl::{DataType, IntervalType, TimeUnit};

assert_eq!(
    DataType::interval(TimeUnit::YearMonth)?.into_arrow_datatype()?,
    ArrowDataType::Interval(IntervalUnit::YearMonth)
);
assert_eq!(
    DataType::from_arrow_datatype(&ArrowDataType::Interval(IntervalUnit::MonthDayNano))?,
    DataType::interval(TimeUnit::MonthDayNano)?
);

// A hand-built leaf its layout does not allow stops at the boundary.
assert!(DataType::Interval(IntervalType::Interval(TimeUnit::Second))
    .into_arrow_datatype()
    .is_err());
```

## No classic spelling

`PT90S` is a [duration](duration.md); an interval stays structural through the
text formats, because months, days and nanoseconds restate none of each other.
A cast into text refuses it, and the shared Arrow text cast leaves an interval
column with Arrow's own rendering.

```rust
use yggdryl::{DataType, Scalar, TimeUnit};

let span = Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano)?;
assert!(DataType::utf8().scalar(span).is_err());

// A duration has one, at the unit's own width.
assert_eq!(
    DataType::utf8().scalar(Scalar::duration64(90, TimeUnit::Second)?)?,
    Scalar::from("PT90S")
);
```

## Edges

- `interval(s)`, `interval(ms)` -> `unit must be an interval layout`, stated under `Interval` by the constructor, `validate` and the root alike.
- `interval(day)` -> `interval requires an interval layout`: the parenthesized day is the scalar unit, and only SQL's bare `interval day` means the layout.
- `Interval::new(1, 0, 0, day_time)` and `Interval::new(1, 2, 0, year_month)` -> refused: a value never claims a layout it does not fit.
- `temporal_count()` answers the nanosecond component; `temporal_count_at` answers `None` for every unit, an interval restating no count.
- An interval into text -> refused; it has no classic spelling, and no [arithmetic](duration.md#arithmetic) with the other temporals either.
- The layout is not part of the identifier: `DataTypeId::Interval` names all three, and the leaf carries which.
- [Merged](../field.md) -> only with its own layout; merged with a [duration](duration.md) -> refused, two families.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::temporal::the_interval_leaf field::temporal::an_interval_field temporal::constructors_reject
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^interval/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_native_scalar.py -k "exact_intervals"
    python/.venv/bin/python -m pytest python/tests/types/test_factories.py -k "typed_factory_parameters"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="exact intervals" node/tests/text/codec.test.js
    node --test --test-name-pattern="typed factory parameters" node/tests/types/fields.test.js
    ```
