# Temporal

Five families - date, time, datetime, duration, interval - each an enum of leaves whose unit is a parameter of the leaf, with the units, the zones and the ISO 8601 spellings they share.

## Contract

| | |
| --- | --- |
| Owns | `DataType::Date(DateType)`, `DataType::Time(TimeType)`, `DataType::DateTime(DateTimeType)`, `DataType::Duration(DurationType)`, `DataType::Interval(IntervalType)`; the values `Date32`, `Date64`, `Time32`, `Time64`, `DateTime64`, `Duration32`, `Duration64`, `Interval`; the `TimeUnit` and [`Timezone`](text.md#time-zones) vocabulary |
| Constructors | one per leaf, validating once - `date32()`, `date64()`, `time32(unit)`, `time64(unit)`, `datetime64(unit, timezone)`, `duration32(unit)`, `duration64(unit)`, `interval(unit)`; `time(unit)` picks the width the unit fits; `time_of(TimeType)` and `duration_of(DurationType)` take a leaf |
| Reads back | `date_type`, `time_type`, `datetime_type`, `duration_type`, `interval_type` answer the family payload, and the payload answers `unit()`, `bit_width()`, `family()`, `validate()`, `id()`; a datetime leaf answers `timezone()` too; `is_temporal` and `kind()` answer over the whole kind |
| Leaf | the storage: `Date32` and `Date64` carry no parameter, `Time32(unit)` and `Time64(unit)` a resolution, `DateTime64 { unit, timezone }` a resolution and a zone, `Duration32(unit)` and `Duration64(unit)` a resolution, `Interval(unit)` a layout; the identifier, the canonical spelling and the schema document are the leaf's, unchanged by the family above it |
| Validates | once, at construction: a width refuses a resolution it does not carry, an interval refuses a resolution, a datetime refuses a layout; a leaf built by hand is caught by `validate` and by the Arrow projection |
| Arrow | `Date32`, `Date64`, `Time32(unit)`, `Time64(unit)`, `Timestamp(unit, zone)` with the zone only when the datatype states one, `Duration(unit)` for both duration widths, `Interval(layout)` |
| Text | the classic ISO 8601 spelling, read and written at the unit's full width, for every family but the interval |
| Rust only | the family enums, the value types, the `Temporal` value enum over the eight leaves and `TemporalValue`; the bindings speak the identifiers, `DataType.time(unit)`, and one factory per leaf |

## Use

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

## Date

A calendar day, in one of two storages. Neither leaf takes a parameter: the
unit is what the leaf is, so `DataType::date32()` and `date64()` are `const`
and `DateType::validate` never refuses.

| leaf | id | holds | unit | Arrow |
| --- | --- | --- | --- | --- |
| `date32` | `0x32` | `i32` days since the Unix epoch | `d` | `Date32` |
| `date64` | `0x33` | `i64` milliseconds since the Unix epoch, at a midnight | `ms` | `Date64` |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `date32` | `Date(DateType::Date32)` | `date` |
| `date64` | `Date(DateType::Date64)` | `date_millisecond` |

Text is `YYYY-MM-DD`, and `YYYYMMDD` reads too; a year outside four digits
has no spelling and keeps its count. A date read under a
[datetime](#datetime) column is that day's midnight. Bindings:
`DataType("date32")`, `types.date32(name)` / `fields.date32(name)`.

```rust
use yggdryl::{DateField, DateType};
use yggdryl::{DataType, Scalar, TimeUnit};

assert_eq!(DataType::date32().date_type(), Some(DateType::Date32));
assert_eq!(DateType::Date64.unit(), TimeUnit::Millisecond);
assert_eq!(DateType::Date64.bit_width(), 64);
assert_eq!(DateType::ALL.map(DateType::family), ["date"; 2]);

// The leaf is the field's payload, and a value knows its width.
let day = DateField::new("day", DateType::Date32, false);
assert_eq!(day.dtype(), &DataType::date32());
assert_eq!(DataType::date64().scalar("1970-01-02")?, Scalar::date64(86_400_000));
assert_eq!(Scalar::from_date(1, TimeUnit::Day, yggdryl::Timezone::NAIVE)?, Scalar::date32(1));
```

## Time

A time of day: a count since midnight at one resolution, in the width that
holds it. The unit is the leaf's parameter, and each width carries two:
`time32` seconds or milliseconds, `time64` microseconds or nanoseconds.
`DataType::time(unit)` is the one rule that picks the width from the unit;
`time32(unit)` and `time64(unit)` name it, and `time_of(TimeType)` takes a
leaf already built.

| leaf | id | holds | unit | Arrow |
| --- | --- | --- | --- | --- |
| `time32(unit)` | 18 | `i32` since midnight | `s`, `ms` | `Time32(unit)` |
| `time64(unit)` | 19 | `i64` since midnight | `us`, `ns` | `Time64(unit)` |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `time32(ms)` | `Time(TimeType::Time32(Millisecond))`; the unit is required | `time(ms)`, `time(3)` |
| `time64(us)` | `Time(TimeType::Time64(Microsecond))`; the unit is required | `time`, `time(us)`, `time(6)` |
| `time(unit)` | the width the unit fits | `time(p)`: a precision `0` is seconds, `1..=3` milliseconds, `4..=6` microseconds, `7..=9` nanoseconds |

Text is `HH:MM:SS[.fraction]`, the fraction at the unit's full width - three
digits are milliseconds, six microseconds - so the digits *are* the unit. An
hour past the day folds into it, `25:30:00` being half past one, and a zone is
refused: a time of day is naive. Bindings: `DataType.time(unit)` in both
languages, `types.time32(name, unit="millisecond")`,
`types.time64(name, unit="microsecond")`, `types.time(name, unit)` and the
same three under `fields`.

```rust
use yggdryl::TimeType;
use yggdryl::{DataType, TimeUnit};

assert_eq!(TimeType::for_unit(TimeUnit::Second)?, TimeType::Time32(TimeUnit::Second));
assert_eq!(DataType::time_of(TimeType::Time64(TimeUnit::Nanosecond))?, DataType::time64(TimeUnit::Nanosecond)?);
assert_eq!(TimeType::ALL, [TimeType::Time32(TimeUnit::Millisecond), TimeType::Time64(TimeUnit::Microsecond)]);
assert_eq!(DataType::from_str("time")?, DataType::time64(TimeUnit::Microsecond)?);
assert_eq!(DataType::from_str("time(3)")?, DataType::time32(TimeUnit::Millisecond)?);
assert_eq!(DataType::time64(TimeUnit::Nanosecond)?.to_string(), "time64(ns)");

// The leaf is public, so a width can be built around a unit it does not
// carry; `validate` and the Arrow projection are where that stops.
let broken = DataType::Time(TimeType::Time32(TimeUnit::Nanosecond));
assert!(broken.validate().is_err());
assert!(broken.into_arrow_datatype().is_err());
```

## Datetime

An instant or a wall-clock reading: one leaf, `datetime64`, carrying its
resolution and its zone. `Timezone::NAIVE` is a wall clock and projects to
Arrow as a timestamp with no zone; any other zone is canonical on arrival and
rides as its name. `DataType::datetime64(unit, timezone)` validates the unit;
the leaf's `with_unit` and `with_timezone` restate one half.

| leaf | id | holds | unit | Arrow |
| --- | --- | --- | --- | --- |
| `datetime64(unit)` | 15 | `i64` since the Unix epoch, a wall clock | `s`, `ms`, `us`, `ns` | `Timestamp(unit, None)` |
| `datetime64(unit,"zone")` | 15 | `i64` since the Unix epoch, UTC | `s`, `ms`, `us`, `ns` | `Timestamp(unit, Some(zone))` |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `datetime64(us)` | naive, microseconds; both parameters default | `datetime64`, `timestamp`, `timestamp_ntz`, `timestamp(6)`, `timestamp without time zone`, `datetime64(us, None)` |
| `datetime64(us,"UTC")` | UTC, microseconds | `datetime64(us, UTC)`, `timestamp_ltz`, `timestamp with time zone`, `datetime64(us, Some(UTC))`, `datetime64(us, timezone=UTC)` |
| `datetime64(ns,"Europe/Paris")` | a named zone | `datetime64(9, Europe/Paris)`, any zone spelling [`Timezone`](text.md#time-zones) canonicalizes |

The zone is quoted in the canonical spelling and may be bare in the grammar; a
precision reads as the unit it needs, as `time(p)` does, and `Some` / `None`
are Arrow's own `Debug` form. Text is `YYYY-MM-DDTHH:MM:SS[.fraction]` for a
wall clock; a zoned instant writes its local reading, the offset that recovers
the instant - `Z` for UTC - and the zone's name in brackets when the name is a
place rather than an offset, because `+02:00` cannot say `Europe/Paris`. A
reading that states no clock is that day's midnight, `20260818Z` in UTC and
`2026-08-18` naive; an hour past the day carries into the next date. Bindings:
`types.datetime64(name, unit="microsecond", timezone="NAIVE")` /
`fields.datetime64(name, unit = 'microsecond', timezone = 'NAIVE')`, and the
[`timezone`](text.md#time-zones) property reads the zone back.

```rust
use yggdryl::DateTimeType;
use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

let naive = DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE)?;
assert_eq!(naive.to_string(), "datetime64(us)");
assert_eq!(DataType::from_str("timestamp")?, naive);
assert_eq!(DataType::from_str("timestamp_ltz")?, DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?);
assert_eq!(DataType::from_str("timestamp(9) with time zone")?.to_string(), "datetime64(ns,\"UTC\")");

let leaf = naive.datetime_type().unwrap();
assert_eq!(leaf, DateTimeType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE });
assert_eq!(leaf.with_timezone(Timezone::UTC).timezone(), Timezone::UTC);
assert_eq!(leaf.with_unit(TimeUnit::Second)?.unit(), TimeUnit::Second);
assert!(leaf.with_unit(TimeUnit::Day).is_err());
assert!(DataType::datetime64(TimeUnit::YearMonth, Timezone::UTC).is_err());

// The count is UTC, as Arrow defines it; the text is the local reading.
let utc = DataType::datetime64(TimeUnit::Second, Timezone::UTC)?;
assert_eq!(utc.scalar("1970-01-01T00:00:10Z")?, Scalar::datetime64(10, TimeUnit::Second, Timezone::UTC)?);
assert_eq!(utc.scalar("19700101Z")?, Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC)?);
assert_eq!(DataType::utf8().scalar(utc.scalar("1970-01-01T00:00:10Z")?)?, Scalar::from("1970-01-01T00:00:10Z"));
```

## Duration

An elapsed count at one resolution, in one of two widths. Both widths carry
the five scalar units, so the unit is the parameter and the width is the
leaf's own choice; Arrow stores every duration in 64 bits, so both project to
`Duration(unit)` and a duration imports as `duration64`.

| leaf | id | holds | unit | Arrow |
| --- | --- | --- | --- | --- |
| `duration32(unit)` | 20 | `i32` elapsed | `d`, `s`, `ms`, `us`, `ns` | `Duration(unit)`; `d` has no Arrow storage |
| `duration64(unit)` | 21 | `i64` elapsed | `d`, `s`, `ms`, `us`, `ns` | `Duration(unit)`; `d` has no Arrow storage |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `duration32(ms)` | `Duration(DurationType::Duration32(Millisecond))`; the unit is required | - |
| `duration64(ns)` | `Duration(DurationType::Duration64(Nanosecond))`; the unit is required | `Duration(ns)`, Arrow's `Debug` form, capital `D` only; lowercase `duration(...)` is not a spelling |

Text is the ISO duration `PT<seconds>[.fraction]S`, the sign leading:
seconds are the one component every unit restates exactly, so the writer
never decomposes into hours a reader would multiply back. Reading takes the
general form `P1DT2H3M4.5S` and a plain clock `-25:30:00.5`, whose hours never
fold. Bindings: `DataType.duration32(unit)` and `DataType.duration64(unit)` in
Python, `types.duration32(name, unit="millisecond")` /
`types.duration64(name, unit="microsecond")` and the same two under `fields`;
JavaScript spells a duration datatype through `DataType.from`.

```rust
use yggdryl::DurationType;
use yggdryl::{DataType, Scalar, TimeUnit};

assert_eq!(DataType::duration32(TimeUnit::Day)?.to_string(), "duration32(d)");
assert_eq!(DataType::duration_of(DurationType::Duration64(TimeUnit::Microsecond))?, DataType::duration64(TimeUnit::Microsecond)?);
assert_eq!(DataType::from_str("Duration(ms)")?, DataType::duration64(TimeUnit::Millisecond)?);
assert!(DataType::from_str("duration(ms)").is_err());
assert!(DataType::duration64(TimeUnit::MonthDayNano).is_err());
// A day count is a duration, but not one Arrow can store.
assert!(DataType::duration64(TimeUnit::Day)?.into_arrow_datatype().is_err());

let seconds = DataType::duration64(TimeUnit::Second)?;
assert_eq!(seconds.scalar("P1DT2H3M4S")?, Scalar::duration64(93_784, TimeUnit::Second)?);
assert_eq!(seconds.scalar("-25:30:00")?, Scalar::duration64(-91_800, TimeUnit::Second)?);
assert_eq!(DataType::utf8().scalar(Scalar::duration32(90, TimeUnit::Second)?)?, Scalar::from("PT90S"));
```

## Interval

A calendar interval in one of Arrow's three layouts. The layout is the
`TimeUnit` parameter - `year_month`, `day_time` or `month_day_nano` - and
`DataType::interval(unit)` refuses a clock resolution, where the bare variant
used to accept one unchecked. The value holds every component of every
layout, and `Interval::new` keeps a layout from claiming a component it does
not have.

| leaf | id | holds | unit | Arrow |
| --- | --- | --- | --- | --- |
| `interval(layout)` | 22 | months, days and nanoseconds; a layout's missing components at zero | `year_month`, `day_time`, `month_day_nano` | `Interval(layout)` |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `interval(month_day_nano)` | the default layout | `interval` |
| `interval(day_time)` | days and milliseconds | SQL's bare `interval day` |
| `interval(year_month)` | whole months | SQL's bare `interval year`, `interval(years)` |

An interval has no classic spelling: `PT90S` is a duration, and an interval
stays structural through the text formats, so a cast into text refuses it.
`Scalar::interval(months, days, nanoseconds, unit)` builds one and
`temporal_count` answers its nanosecond component. Bindings:
`types.interval(name, unit="month_day_nano")` /
`fields.interval(name, unit = 'month_day_nano')`.

```rust
use yggdryl::{Interval, IntervalType};
use yggdryl::{DataType, Scalar, TimeUnit};

let span = DataType::interval(TimeUnit::DayTime)?;
assert_eq!(span.interval_type(), Some(IntervalType::Interval(TimeUnit::DayTime)));
assert_eq!(DataType::from_str("interval")?, DataType::interval(TimeUnit::MonthDayNano)?);
assert_eq!(DataType::from_str("interval day")?, span);
assert!(DataType::from_str("interval(day)").is_err());
assert!(DataType::interval(TimeUnit::Second).is_err());

// Every layout is months, days and nanoseconds; a layout refuses a component it lacks.
let value = Scalar::interval(1, 2, 3, TimeUnit::MonthDayNano)?;
assert_eq!(value.temporal_count(), Some(3));
assert_eq!(value.temporal_dtype(), Some(DataType::interval(TimeUnit::MonthDayNano)?));
assert!(Interval::new(1, 0, 0, TimeUnit::DayTime).is_err());
assert!(DataType::utf8().scalar(value).is_err());
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
| `duration32(unit)`, `duration64(unit)` | `Duration(unit)` | `duration64(unit)` |
| `interval(layout)` | `Interval(layout)` | `interval(layout)` |

## Vocabulary

Units parse case-insensitively; whitespace, `_`, or `-` may separate words.

| Canonical | Also accepted | Accepted by |
| --- | --- | --- |
| `d` | `day`, `days` | `duration32`, `duration64` |
| `s` | `sec`, `secs`, `second`, `seconds` | `time32`, `datetime64`, `duration32`, `duration64` |
| `ms` | `milli`, `millis`, `millisecond`, `milliseconds` | `time32`, `datetime64`, `duration32`, `duration64` |
| `us` | `µs`, `micro`, `micros`, `microsecond`, `microseconds` | `time64`, `datetime64`, `duration32`, `duration64` |
| `ns` | `nano`, `nanos`, `nanosecond`, `nanoseconds` | `time64`, `datetime64`, `duration32`, `duration64` |
| `year_month` | `year`, `years`, `yearmonth`, `year_to_month`, `years_to_months` | `interval` |
| `day_time` | `daytime`, `day_to_time`, `days_to_time`, `day_to_second`, `days_to_seconds` | `interval` |
| `month_day_nano` | `monthdaynano`, `monthdaynanos`, `month_day_nanosecond`, `month_day_nanoseconds` | `interval` |

`TimeUnit` is the one unit parser and Arrow converter (see [Scalar](scalar.md)
for the shared enums); a temporal `Scalar` answers `temporal_unit`,
`temporal_timezone` and `temporal_count` across the widths, and `as_temporal`
the `Temporal` value whose `family()` names the family it belongs to.

### Timezone

| Spelling | Canonical value |
| --- | --- |
| `Asia/Calcutta`, `US/Eastern` | `Asia/Kolkata`, `America/New_York` (`key` in Python and JavaScript) |
| `UTC`, `utc`, `Z`, `GMT`, `Etc/UTC`, `+00:00` | `UTC` (`is_utc`, `is_fixed`) |
| `+0530`, `from_offset(19800)`, a fixed-offset `tzinfo` | `+05:30` (`is_fixed`) |
| `zoneinfo.ZoneInfo("Europe/Paris")` (Python) | `Europe/Paris` (`observes_saving`) |
| `Timezone::NAIVE` | wall clock; `is_naive`, projects to Arrow as no timezone |

`offset_at`, `abbreviation_at` and `is_saving_at` follow the bundled registry;
the value itself is documented with the [text values](text.md#time-zones).

## Edges

- `time32(ns)` -> `unit must be second or millisecond`; `time64(s)` -> `unit must be microsecond or nanosecond`.
- `time(d)`, `time(year_month)`, `time(day_time)`, `time(month_day_nano)` -> `unit must be a temporal resolution`.
- `time("fortnight")` -> `unknown temporal resolution`; Python `DataType.time(1)` -> `TypeError`; JavaScript `DataType.time()` throws.
- `datetime64(d, tz)`, `datetime64(year_month, tz)` -> `unit must be a temporal resolution`, under the kind `datetime64`; `DataType::validate` on a hand-built leaf says the same.
- `duration32(year_month)` -> `unit must be day, second, millisecond, microsecond, or nanosecond`; `duration64(d)` is a datatype but has no Arrow storage, so the projection refuses it.
- `interval(s)` -> `unit must be an interval layout`; the grammar's `interval(day)` -> `interval requires an interval layout`, because the parenthesized day is the scalar unit and only SQL's bare `interval day` means the layout.
- `Time32(Nanosecond)` built directly -> `validate`, `into_arrow_datatype`, `into_arrow_datatype_ffi` fail; `DataType::time32` refuses.
- Arrow `Duration(unit)` -> `duration64(unit)`; the 32-bit width widens on the way out and never comes back.
- A time of day or a duration with a zone in its text -> refused; both are naive, and a datetime is where a zone belongs.
- `2026-02-30` -> refused, not a guess; `25:30:00` as a time of day is `01:30:00`, as a datetime the next day at `01:30`, as a duration twenty-five and a half hours.
- An interval into text -> refused; it has no classic spelling.
- `Timezone("")`, `Timezone("+25:00")` -> `ValueError` (JavaScript throws); `Timezone.fromOffset(25 * 3600)` throws; Python `Timezone(object())` -> `TypeError`.
- [Merged](field.md) within one family -> one unit, `date64` over `date32`, `duration64` over `duration32`, `time` at the unit picking its width, and a zone one side declares kept; two families -> refused, a date beside a datetime included.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::temporal field::temporal temporal:: timezone::
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- timezone::tests
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^(date|time|datetime|duration|interval|time_unit|temporal_text)/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "time_infers"
    python/.venv/bin/python -m pytest python/tests/types/test_timezone.py
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="generic time|temporal and decimal" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    node --test node/tests/types/timezone.test.js
    ```
