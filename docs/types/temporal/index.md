# Temporal

Five families - date, time, datetime, duration and interval - eight leaves over one unit vocabulary and one zone value.

## Contract

| | |
| --- | --- |
| Owned | `DataType::Date(DateType)`, `Time(TimeType)`, `DateTime(DateTimeType)`, `Duration(DurationType)`, `Interval(IntervalType)`; the values `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64`, `Duration32`, `Duration64`, `Interval`; `Temporal`, the one value enum over the eight leaves; the `TimeUnit` vocabulary and the [`Timezone`](timezone.md) value |
| Validated | once, at construction: a width refuses a resolution it does not carry, an interval refuses a resolution, a datetime refuses a layout; a leaf built by hand is caught by `validate` and again by the Arrow projection |
| Lazy | nothing; every leaf is `Copy` and every value is a count, a unit and a zone |
| Cached | the [field](../field.md)'s Arrow projection, as every field caches it; a datatype caches nothing |
| Refused | a unit no family counts in, a zone on a date, a time of day or a duration, a calendar date that does not exist, an interval rendered as text, and a merge across two families |

## Pages

| page | owns |
| --- | --- |
| [Date](date.md) | `DateType`: `date32` days since the epoch, `date64` the milliseconds of a midnight, no parameter |
| [Time](time.md) | `TimeType`: `time32(unit)` and `time64(unit)`, the resolution picking the width, SQL's `time(p)` |
| [Datetime](datetime.md) | `DateTimeType`: the one `datetime64 { unit, timezone }` leaf and every `timestamp` spelling |
| [Duration](duration.md) | `DurationType`: `duration32(unit)` and `duration64(unit)` over one Arrow width |
| [Interval](interval.md) | `IntervalType`: `interval(layout)` over Arrow's three layouts, and the value holding all three components |
| [Time zone](timezone.md) | the `Timezone` value, the bundled IANA registry, and the `timezone` datatype a column of zones declares |

## The five families

The leaf is the storage and its unit the parameter; the family is what a reader matches on.

=== "Rust"

    ```rust
    use arrow_schema::{DataType as ArrowDataType, IntervalUnit, TimeUnit as ArrowTimeUnit};
    use yggdryl::{DateTimeType, DateType, DurationType, IntervalType, TimeType};
    use yggdryl::{DataType, DataTypeId, DataTypeKind, Scalar, Temporal, TimeUnit, Timezone};

    // Five families; each constructor picks a leaf and validates its unit once.
    let day = DataType::date32();
    let clock = DataType::time32(TimeUnit::Millisecond)?;
    let at = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?;
    let elapsed = DataType::duration64(TimeUnit::Nanosecond)?;
    let span = DataType::interval(TimeUnit::MonthDayNano)?;

    assert_eq!(day, DataType::Date(DateType::Date32));
    assert_eq!(clock, DataType::Time(TimeType::Time32(TimeUnit::Millisecond)));
    assert_eq!(
        at,
        DataType::DateTime(DateTimeType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::UTC })
    );
    assert_eq!(elapsed, DataType::Duration(DurationType::Duration64(TimeUnit::Nanosecond)));
    assert_eq!(span, DataType::Interval(IntervalType::Interval(TimeUnit::MonthDayNano)));

    // The identifier, the spelling and the grammar are the leaf's.
    assert_eq!(day.id(), DataTypeId::Date32);
    assert_eq!(clock.to_string(), "time32(ms)");
    assert_eq!(at.to_string(), "datetime64(us,\"UTC\")");
    assert_eq!(DataType::from_str("datetime64(us, UTC)")?, at);
    assert_eq!(DataType::from_str("date")?, day);
    assert_eq!(DataType::from_str("time(ms)")?, clock);
    assert_eq!(elapsed.kind(), DataTypeKind::Temporal);

    // The payload reads back, with its unit, its width and its zone.
    assert_eq!(clock.time_type().unwrap().unit(), TimeUnit::Millisecond);
    assert_eq!(clock.time_type().unwrap().bit_width(), 32);
    assert_eq!(at.datetime_type().unwrap().timezone(), Timezone::UTC);
    assert_eq!(span.interval_type().unwrap().unit(), TimeUnit::MonthDayNano);
    assert_eq!(elapsed.duration_type().unwrap().family(), "duration");
    assert_eq!(day.time_type(), None);

    // A value answers the same family, as one `Temporal` over the eight leaves.
    let ninety = Scalar::duration64(90, TimeUnit::Second)?;
    assert!(matches!(ninety.as_temporal(), Some(Temporal::Duration64(_))));
    assert_eq!(ninety.as_temporal().map(|held| held.family()), Some("duration"));

    // `time` picks the width from the unit; a unit a width does not carry is refused.
    assert_eq!(DataType::time(TimeUnit::Second)?, DataType::time32(TimeUnit::Second)?);
    assert_eq!(DataType::time(TimeUnit::Nanosecond)?, DataType::time64(TimeUnit::Nanosecond)?);
    assert!(DataType::time32(TimeUnit::Nanosecond).is_err());
    assert!(DataType::time(TimeUnit::YearMonth).is_err());
    assert!(DataType::interval(TimeUnit::Second).is_err());

    // Arrow storage, and the import back: Arrow has one duration width.
    assert_eq!(day.clone().into_arrow_datatype()?, ArrowDataType::Date32);
    assert_eq!(
        at.clone().into_arrow_datatype()?,
        ArrowDataType::Timestamp(ArrowTimeUnit::Microsecond, Some("UTC".into()))
    );
    assert_eq!(elapsed.clone().into_arrow_datatype()?, ArrowDataType::Duration(ArrowTimeUnit::Nanosecond));
    assert_eq!(span.clone().into_arrow_datatype()?, ArrowDataType::Interval(IntervalUnit::MonthDayNano));
    assert_eq!(
        DataType::from_arrow_datatype(&ArrowDataType::Duration(ArrowTimeUnit::Millisecond))?,
        DataType::duration64(TimeUnit::Millisecond)?
    );

    // Text reads and writes the classic ISO spelling, at the unit's width.
    assert_eq!(day.scalar("1970-01-02")?, Scalar::date32(1));
    assert_eq!(clock.scalar("10:15:30.5")?, Scalar::time32(36_930_500, TimeUnit::Millisecond, Timezone::NAIVE)?);
    assert_eq!(DataType::utf8().scalar(Scalar::duration64(90, TimeUnit::Second)?)?, Scalar::from("PT90S"));
    ```

=== "Python"

    ```python
    import datetime

    import pyarrow as pa
    import pytest

    from yggdryl import DataType, Field, types

    # Five families; the spelling is the leaf's, and `time` picks the width.
    assert DataType("time32(ms)") == DataType.time("ms")
    assert DataType.time("ns") == DataType("time64(ns)")
    assert DataType("date") == DataType("date32")
    assert DataType("timestamp") == DataType("datetime64(us)")
    assert DataType("timestamp_ltz") == DataType('datetime64(us,"UTC")')
    assert DataType("interval") == DataType("interval(month_day_nano)")

    # One factory per leaf; a datetime states its unit and its zone.
    at = types.datetime64("at", "us", "UTC")
    assert str(at.dtype) == 'datetime64(us,"UTC")'
    assert str(at.dtype.timezone) == "UTC"
    assert at.dtype.kind == "temporal"
    assert at.into_arrow().type == pa.timestamp("us", tz="UTC")

    # Arrow has one duration width, so a duration imports as `duration64`.
    assert DataType("duration32(ms)").into_arrow() == pa.duration("ms")
    assert DataType.from_arrow(pa.duration("ns")) == DataType("duration64(ns)")

    # The core value path reads the classic ISO spelling.
    day = Field("day", "date32", nullable=False)
    assert day.scalar("1970-01-02").as_py() == datetime.date(1970, 1, 2)
    assert day.scalar("1970-01-02").into_arrow_scalar() == pa.scalar(1, pa.date32())

    with pytest.raises(ValueError, match="temporal resolution"):
        DataType.time("year_month")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    // Five families; the spelling is the leaf's, and `time` picks the width.
    assert.equal(DataType.time('ms').toString(), 'time32(ms)')
    assert.equal(DataType.time('ns').toString(), 'time64(ns)')
    assert.ok(DataType.from('date').equals(DataType.from('date32')))
    assert.ok(DataType.from('timestamp').equals(DataType.from('datetime64(us)')))
    assert.ok(DataType.from('interval').equals(DataType.from('interval(month_day_nano)')))

    // One factory per leaf; a datetime states its unit and its zone.
    const at = fields.datetime64('at', 'us', 'UTC')
    assert.equal(at.dtype.toString(), 'datetime64(us,"UTC")')
    assert.ok(DataType.from('datetime64(us,UTC)').equals(at.dtype))
    assert.ok(DataType.from('timestamp_ltz').equals(at.dtype))
    assert.equal(at.dtype.id, 'datetime64')
    assert.equal(at.dtype.kind, 'temporal')

    // The unit is the factory's parameter, defaulted where the leaf has one.
    assert.equal(fields.date32('day').dtype.id, 'date32')
    assert.equal(fields.time32('clock', 's').dtype.toString(), 'time32(s)')
    assert.equal(fields.duration64('elapsed', 'ns').dtype.toString(), 'duration64(ns)')
    assert.ok(fields.duration32('elapsed').dtype.equals(DataType.from('duration32(ms)')))
    assert.equal(fields.interval('span').dtype.toString(), 'interval(month_day_nano)')

    assert.throws(() => DataType.time('year_month'), /temporal resolution/)
    ```

## One value across the families

`Temporal` is the one value enum over the eight leaves, and `family()` answers
`date`, `time`, `datetime`, `duration` or `interval` - the same word a datatype
payload answers. The [`Scalar`](../scalar.md) readers go through it, so a count,
a unit and a zone are read without naming a width.

=== "Rust"

    ```rust
    use yggdryl::{DataType, FamilyValue, Scalar, Temporal, TimeUnit, Timezone};

    let values = [
        Scalar::date32(1),
        Scalar::time32(1, TimeUnit::Second, Timezone::NAIVE)?,
        Scalar::datetime64(1, TimeUnit::Nanosecond, Timezone::UTC)?,
        Scalar::duration64(1, TimeUnit::Microsecond)?,
        Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano)?,
    ];
    assert!(values.iter().all(Scalar::is_temporal));
    assert_eq!(
        values
            .iter()
            .map(|value| value.as_temporal().map(|held| held.family()))
            .collect::<Vec<_>>(),
        vec![Some("date"), Some("time"), Some("datetime"), Some("duration"), Some("interval")],
    );

    // Count, unit and zone answer across the widths.
    assert_eq!(values[2].temporal_count(), Some(1));
    assert_eq!(values[2].temporal_unit(), Some(TimeUnit::Nanosecond));
    assert_eq!(values[2].temporal_timezone(), Some(Timezone::UTC));
    assert_eq!(values[3].temporal_count_at(TimeUnit::Nanosecond), Some(1_000));
    // An interval restates no count across units: a month is no count of days.
    assert_eq!(values[4].temporal_count_at(TimeUnit::Nanosecond), None);

    // The family value is one enum, and it answers the leaf's own datatype.
    let held = Temporal::from_scalar(&values[2]).expect("a datetime");
    assert_eq!(held.dtype()?, DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?);

    // A number is no temporal, and every reader says so the same way.
    let number = Scalar::from(1_i64);
    assert!(!number.is_temporal());
    assert_eq!(number.temporal_unit(), None);
    assert_eq!(Temporal::from_scalar(&number), None);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    # The same three readers, spelled as properties.
    instant = DataType('datetime64(us,"UTC")').scalar(0)
    assert instant.family == "temporal"
    assert instant.id == "datetime64"
    assert instant.count == 0
    assert instant.unit == "us"
    assert instant.zone == "UTC"

    # Every temporal carries a zone; only a datetime can carry one that is not
    # the zone-free marker.
    assert DataType("date32").scalar(1).zone == "NAIVE"
    assert Scalar.duration(1, "ms").zone == "NAIVE"

    # A value outside the kind answers nothing rather than guessing.
    assert Scalar.from_(1).count is None
    assert Scalar.from_(1).unit is None
    assert Scalar.from_(1).zone is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    const values = [
      [new DataType('date32').scalar(7), 'date32', 7n, 'd'],
      [new DataType('date64').scalar(86_400_000n), 'date64', 86_400_000n, 'ms'],
      [new DataType('time32(s)').scalar(7), 'time32', 7n, 's'],
      [new DataType('time64(us)').scalar(7n), 'time64', 7n, 'us'],
      [new DataType('datetime64(ns)').scalar(7n), 'datetime64', 7n, 'ns'],
      [Scalar.duration(7, 'ms'), 'duration32', 7n, 'ms'],
      [Scalar.duration(2147483648n, 'us'), 'duration64', 2147483648n, 'us'],
    ]
    for (const [value, kind, count, unit] of values) {
      assert.equal(value.kind, kind)
      assert.equal(value.family, 'temporal')
      assert.equal(value.count, count)
      assert.equal(value.unit, unit)
      assert.equal(value.zone, 'NAIVE')
    }

    // A zone belongs to the datetime, and is read back off the value.
    assert.equal(new DataType('datetime64(ns,"UTC")').scalar(7n).zone, 'UTC')
    ```

## Arrow storage

| datatype | Arrow | imports back as |
| --- | --- | --- |
| `date32` | `Date32` | `date32` |
| `date64` | `Date64` | `date64` |
| `time32(unit)` | `Time32(unit)` | `time32(unit)` |
| `time64(unit)` | `Time64(unit)` | `time64(unit)` |
| `datetime64(unit)` | `Timestamp(unit, None)` | `datetime64(unit)` |
| `datetime64(unit,"zone")` | `Timestamp(unit, Some("zone"))` | `datetime64(unit,"zone")`, the zone canonical |
| `duration32(unit)`, `duration64(unit)` | `Duration(unit)`; `d` has no Arrow storage | `duration64(unit)` |
| `interval(layout)` | `Interval(layout)` | `interval(layout)` |

One Arrow cast serves every family: a temporal column renders as the classic
text this crate spells - which is why a zoned instant renders here where
Arrow's own formatter refuses one - and a text column is read back under the
exact column's unit and zone. An [interval](interval.md) has no classic
spelling and keeps Arrow's rendering.

## Vocabulary

Units parse case-insensitively; whitespace, `_`, or `-` may separate words.
The constructors take every unit its family carries; the grammar's unit
parameter takes an Arrow resolution only, which is where the two columns
below differ.

| Canonical | Also accepted | Taken by | Read back by the grammar |
| --- | --- | --- | --- |
| `d` | `day`, `days` | `duration32`, `duration64` | nothing: `duration32(d)` renders but does not read back |
| `s` | `sec`, `secs`, `second`, `seconds` | `time32`, `datetime64`, `duration32`, `duration64` | the same four |
| `ms` | `milli`, `millis`, `millisecond`, `milliseconds` | `time32`, `datetime64`, `duration32`, `duration64` | the same four |
| `us` | `µs`, `micro`, `micros`, `microsecond`, `microseconds` | `time64`, `datetime64`, `duration32`, `duration64` | the same four |
| `ns` | `nano`, `nanos`, `nanosecond`, `nanoseconds` | `time64`, `datetime64`, `duration32`, `duration64` | the same four |
| `year_month` | `year`, `years`, `yearmonth`, `year_to_month`, `years_to_months` | `interval` | `interval` |
| `day_time` | `daytime`, `day_to_time`, `days_to_time`, `day_to_second`, `days_to_seconds` | `interval` | `interval` |
| `month_day_nano` | `monthdaynano`, `monthdaynanos`, `month_day_nanosecond`, `month_day_nanoseconds` | `interval` | `interval` |

`TimeUnit` is the one unit parser and Arrow converter in the whole crate (the
shared enums are listed on [Scalar](../scalar.md)); `date32` and `date64` state
no unit because the width is the unit, and every other family takes one as its
leaf's parameter. The zone vocabulary is one value, documented on
[Time zone](timezone.md).

## Edges

- A reading that states no clock is that day's midnight, so a date sits in the same column a transact time does; `25:30:00` is `01:30:00` as a time of day, the next day at `01:30` as a datetime, and twenty-five and a half hours as a duration.
- `2026-02-30` -> refused, not a guess; so is a fraction with no digits after its decimal sign, and a compact reading of the wrong width.
- ISO 8601 names the comma the preferred decimal sign, so `01,148` and `01.148` read one value in every spelling that carries a fraction.
- [Merged](../field.md) within one family -> one unit, `date64` over `date32`, `duration64` over `duration32`, `time` at the unit picking its width, and a zone one side declares kept; two families -> refused, a date beside a datetime included.
- The family enums, the value types, the `Temporal` enum and `TemporalValue` are Rust only; the bindings speak the identifiers, `DataType.time(unit)`, and one factory per leaf.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::temporal field::temporal temporal:: timezone::
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^(date|time|datetime|duration|interval|time_unit|temporal_text)/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "time_infers"
    python/.venv/bin/python -m pytest python/tests/types/test_native_scalar.py -k "width_unit_scale_and_zone or exact_intervals"
    python/.venv/bin/python -m pytest python/tests/types/test_timezone.py
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="generic time|temporal families" node/tests/types/datatype.test.js node/tests/text/codec.test.js
    node --test node/tests/types/timezone.test.js
    ```
