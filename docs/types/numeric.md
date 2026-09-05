# Numeric & temporal

This page owns the boolean, integer, floating, decimal, and temporal datatypes, their width selectors, and the unit and zone vocabulary.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `null`, `boolean`, `int8`..`int64`, `uint8`..`uint64`, `float16`/`float32`/`float64`, `decimal32`..`decimal256`, `date32`, `date64`, `time32`, `time64`, `datetime64`, `duration32`, `duration64`, `interval` |
| Selectors | `decimal(p, s)` and `time(unit)` pick a physical width; the exact constructors (`decimal32`..`decimal256`, `time32`, `time64`) stay available |
| Validates | Once at construction: precision `1..=max` of the width, positive scale `<= precision`, unit category per type; a bad parameter never becomes a value |
| Errors | Rust `Error::InvalidDataType { kind, reason }`; Python `ValueError` (`TypeError`/`OverflowError` for bad argument types); JavaScript throws |
| `datetime64(unit, tz)` | Unit `s`/`ms`/`us`/`ns` plus a `Timezone`; `Timezone::NAIVE` is wall clock and projects to Arrow as no timezone; `timestamp` spellings parse the same |
| Durations | `duration32`/`duration64` take `d`/`s`/`ms`/`us`/`ns`; Arrow import always lands on `duration64` |
| `interval` | `year_month`, `day_time`, or `month_day_nano`; never a time-of-day resolution |
| `TimeUnit` | Shared enum (see [Scalar](scalar.md)); ASCII case-insensitive parse, canonical short display |
| `Timezone` | Canonical on arrival: aliases resolve, offsets normalize to `+HH:MM`, every UTC spelling is `UTC`; `offset_at`, `abbreviation_at`, `is_saving_at` follow the bundled registry |
| Bindings | Python `DataType.decimal(precision, scale=0)`, `DataType.time(unit)`; JavaScript `DataType.time(unit)` and `fields.decimal*(name, precision, scale=0)`; no `DataType.decimal` in JavaScript |

## Use

The selectors validate once and pick the physical width at construction.

=== "Rust"

    ```rust
    use yggdryl::{DataType, TimeUnit};

    assert_eq!(DataType::decimal(38, 4)?, DataType::decimal128(38, 4)?);
    assert_eq!(DataType::decimal(39, 4)?, DataType::decimal256(39, 4)?);
    assert_eq!(DataType::time(TimeUnit::Second)?, DataType::Time32(TimeUnit::Second));
    assert_eq!(DataType::time(TimeUnit::Nanosecond)?, DataType::Time64(TimeUnit::Nanosecond));

    // Out-of-range parameters and mismatched unit categories are refused.
    assert!(DataType::decimal(2, 3).is_err());
    assert!(DataType::decimal128(39, 0).is_err());
    assert!(DataType::time32(TimeUnit::Nanosecond).is_err());
    assert!(DataType::time(TimeUnit::YearMonth).is_err());
    assert!(DataType::fixed_size_binary(-1).is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    assert DataType.decimal(38, 4) == DataType("decimal128(38,4)")
    assert DataType.decimal(39, 4) == DataType("decimal256(39,4)")
    assert DataType.time("s") == DataType("time32(s)")
    assert DataType.time("nano seconds") == DataType("time64(ns)")

    with pytest.raises(ValueError, match="positive scale cannot exceed precision"):
        DataType.decimal(2, 3)
    with pytest.raises(ValueError, match="temporal resolution"):
        DataType.time("year_month")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    assert.equal(fields.decimal('amount', 38, 4).dtype.toString(), 'decimal128(38,4)')
    assert.equal(fields.decimal('wide', 39, 4).dtype.toString(), 'decimal256(39,4)')
    assert.equal(DataType.time('s').toString(), 'time32(s)')
    assert.equal(DataType.time('nano seconds').toString(), 'time64(ns)')

    assert.throws(() => fields.decimal('bad', 2, 3), /positive scale cannot exceed precision/)
    assert.throws(() => DataType.time('year_month'), /temporal resolution/)
    ```

## Selectors

| Call | Result |
| --- | --- |
| `decimal(1..=9, s)` | `decimal32(p,s)` |
| `decimal(10..=18, s)` | `decimal64(p,s)` |
| `decimal(19..=38, s)` | `decimal128(p,s)` |
| `decimal(39..=76, s)` | `decimal256(p,s)` |
| `numeric(p,s)` (parser) | same as `decimal(p,s)`; `bignumeric` is `decimal256` |
| `time(s)`, `time(ms)` | `time32(s)`, `time32(ms)` |
| `time(us)`, `time(ns)` | `time64(us)`, `time64(ns)` |
| `time` (parser, no unit) | `time64(us)` |
| `Duration(unit)` (Arrow `Debug` form) | `duration64(unit)`; lowercase `duration(...)` is not a spelling |

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

### Timezone

| Spelling | Canonical value |
| --- | --- |
| `Asia/Calcutta`, `US/Eastern` | `Asia/Kolkata`, `America/New_York` (`key` in Python and JavaScript) |
| `UTC`, `utc`, `Z`, `GMT`, `Etc/UTC`, `+00:00` | `UTC` (`is_utc`, `is_fixed`) |
| `+0530`, `from_offset(19800)`, a fixed-offset `tzinfo` | `+05:30` (`is_fixed`) |
| `zoneinfo.ZoneInfo("Europe/Paris")` (Python) | `Europe/Paris` (`observes_saving`) |
| `Timezone::NAIVE` | wall clock; `is_naive`, projects to Arrow as no timezone |

## Typed markers

Rust only. Each marker `M` has a `TypedField<M>` alias (`Int8Field`, `Decimal128Field`, ...) described on [Field](field.md).

| Family (`yggdryl::types::<family>`) | Markers |
| --- | --- |
| `boolean` | `NullType`, `BooleanType` |
| `integer` | `Int8Type`, `Int16Type`, `Int32Type`, `Int64Type`, `UInt8Type`, `UInt16Type`, `UInt32Type`, `UInt64Type` |
| `floating` | `Float16Type`, `Float32Type`, `Float64Type` |
| `decimal` | `Decimal32Type`, `Decimal64Type`, `Decimal128Type`, `Decimal256Type` |
| `temporal` | `Date32Type`, `Date64Type`, `Time32Type`, `Time64Type`, `DateTime64Type`, `Duration32Type`, `Duration64Type`, `IntervalType` |

## Edges

- `decimal(2, 3)` -> refused, `positive scale cannot exceed precision: 3 > 2`; a negative scale is accepted (`decimal256(39,-4)`).
- `decimal(0, 0)` -> `precision must be between 1 and 9: 0`; `decimal128(39, 0)` -> `precision must be between 1 and 38: 39`; `decimal(77, 0)` -> the `1 and 76` bound of `decimal256`.
- `time32(ns)` -> `unit must be second or millisecond`; `time64(s)` -> `unit must be microsecond or nanosecond`.
- `time(d)`, `time(year_month)`, `time(day_time)`, `time(month_day_nano)` -> `unit must be a temporal resolution`.
- `time("fortnight")` -> `unknown temporal resolution`; Python `DataType.time(1)` -> `TypeError`; JavaScript `DataType.time()` throws.
- `datetime64(d, tz)` -> `unit must be second, millisecond, microsecond, or nanosecond`.
- `duration32(year_month)` -> `unit must be day, second, millisecond, microsecond, or nanosecond`.
- Python `DataType.decimal(True, 0)` -> `TypeError`; `DataType.decimal("18.0", 2)` -> `ValueError`; `DataType.decimal(256, 0)` -> `OverflowError`; `__index__` objects and base-10 strings are accepted.
- `Timezone("")`, `Timezone("+25:00")` -> `ValueError` (JavaScript throws); `Timezone.fromOffset(25 * 3600)` throws; Python `Timezone(object())` -> `TypeError`.
- `fixed_size_binary(-1)` -> refused; the binary widths live on [Text & bytes](text.md).

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::floating datatype::temporal field::integer field::floating field::decimal field::temporal field::scalar
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- types::decimal types::temporal types::arithmetic
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^decimal/'
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^time(_unit)?/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_datatype.py -k "decimal or time_infers"
    python/.venv/bin/python -m pytest python/tests/types/test_timezone.py
    python/.venv/bin/python python/benchmarks/types.py --iterations 10000
    python/.venv/bin/python python/benchmarks/types/scalars.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="generic time|temporal and decimal" node/tests/types/datatype.test.js node/tests/types/fields.test.js
    node --test node/tests/types/timezone.test.js
    npm run --prefix node bench:types
    ```
