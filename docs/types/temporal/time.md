# Time

A time of day: a count since midnight at one resolution, in the width that holds it.

## Contract

| | |
| --- | --- |
| Owned | `TimeType` with its two leaves, `DataType::Time32(TimeUnit)` and `DataType::Time64(TimeUnit)`, the `TimeField` marker, and the `Time32` and `Time64` values |
| Validated | once, at construction: `time32` carries seconds or milliseconds and `time64` microseconds or nanoseconds, and `TimeType::for_unit` is the one rule that maps a resolution to a width. A leaf built by hand is caught by `validate` and by the Arrow projection |
| Lazy | nothing; the leaf is `Copy` and the value is a count, a unit and a zone |
| Cached | the [field](../field.md)'s Arrow projection; the datatype caches nothing |
| Refused | a resolution the width does not carry, a day or an interval layout, a count outside its day, and any zone - a time of day is naive |

## DataType

The unit is the leaf's parameter, and each width carries two of them.
`DataType::time(unit)` is the one rule that picks the width from the unit;
`time32(unit)` and `time64(unit)` name it, and `time_of(TimeType)` takes a leaf
already built.

| leaf | `DataTypeId` | `as_u8()` | holds | unit | Arrow |
| --- | --- | --- | --- | --- | --- |
| `time32(unit)` | `Time32` | `0x34` | `i32` since midnight | `s`, `ms` | `Time32(unit)` |
| `time64(unit)` | `Time64` | `0x35` | `i64` since midnight | `us`, `ns` | `Time64(unit)` |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `time32(ms)` | `DataType::Time32(Millisecond)`; the unit is required | `time(ms)`, `time(3)` |
| `time64(us)` | `DataType::Time64(Microsecond)`; the unit is required | `time`, `time(us)`, `time(6)` |
| `time(unit)` | the width the unit fits | `time(p)`: a precision `0` is seconds, `1..=3` milliseconds, `4..=6` microseconds, `7..=9` nanoseconds |

FIX's `UTCTimeOnly` and `LocalMktTime` are both `time64(ns)`: a time of day
with a fraction is one type whichever field carried it, and the
[datatype page](../datatype.md) lists the whole logical vocabulary.

=== "Rust"

    ```rust
    use yggdryl::TimeType;
    use yggdryl::{DataType, DataTypeId, DataTypeKind, TimeUnit};

    // The resolution picks the width; the explicit constructors name it.
    assert_eq!(TimeType::for_unit(TimeUnit::Second)?, TimeType::Time32(TimeUnit::Second));
    assert_eq!(DataType::time(TimeUnit::Second)?, DataType::time32(TimeUnit::Second)?);
    assert_eq!(DataType::time(TimeUnit::Nanosecond)?, DataType::time64(TimeUnit::Nanosecond)?);
    assert_eq!(
        DataType::time_of(TimeType::Time64(TimeUnit::Nanosecond))?,
        DataType::time64(TimeUnit::Nanosecond)?
    );
    assert_eq!(
        TimeType::ALL,
        [TimeType::Time32(TimeUnit::Millisecond), TimeType::Time64(TimeUnit::Microsecond)]
    );

    // The leaf answers its own width, family and identifier.
    let leaf = DataType::time64(TimeUnit::Nanosecond)?.time_type().expect("a time datatype");
    assert_eq!(leaf, TimeType::Time64(TimeUnit::Nanosecond));
    assert_eq!(leaf.unit(), TimeUnit::Nanosecond);
    assert_eq!(leaf.bit_width(), 64);
    assert_eq!(leaf.id().temporal_family(), Some("time"));
    assert_eq!(leaf.id(), DataTypeId::Time64);
    assert_eq!(TimeType::from_id(DataTypeId::Time32, TimeUnit::Second), Some(TimeType::Time32(TimeUnit::Second)));
    assert_eq!(DataTypeId::Time32.as_u8(), 0x34);
    assert!(DataTypeId::Time64.is_parameterized());

    // The grammar, including SQL's precision.
    assert_eq!(DataType::from_str("time")?, DataType::time64(TimeUnit::Microsecond)?);
    assert_eq!(DataType::from_str("time(3)")?, DataType::time32(TimeUnit::Millisecond)?);
    assert_eq!(DataType::from_str("time(9)")?, DataType::time64(TimeUnit::Nanosecond)?);
    assert_eq!(DataType::from_str("time(micro seconds)")?, DataType::time64(TimeUnit::Microsecond)?);
    assert_eq!(DataType::time64(TimeUnit::Nanosecond)?.to_string(), "time64(ns)");
    assert_eq!(DataType::time32(TimeUnit::Second)?.kind(), DataTypeKind::Temporal);

    // A width refuses a resolution it does not carry, and a layout is no clock.
    assert!(DataType::time32(TimeUnit::Nanosecond).is_err());
    assert!(DataType::time64(TimeUnit::Second).is_err());
    assert!(DataType::time(TimeUnit::Day).is_err());
    assert!(DataType::from_str("time(year_month)").is_err());
    assert!(DataType::from_str("time(10)").is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType

    # `time` picks the width from the unit; the leaves name it.
    assert DataType.time("ms") == DataType("time32(ms)")
    assert DataType.time("ns") == DataType("time64(ns)")
    assert DataType("time") == DataType("time64(us)")
    assert DataType("time(3)") == DataType("time32(ms)")
    assert DataType("time32(s)").id == "time32"
    assert DataType("time64(ns)").kind == "temporal"

    # Arrow's two clock storages, and the import back.
    assert DataType("time32(ms)").into_arrow() == pa.time32("ms")
    assert DataType.from_arrow(pa.time64("ns")) == DataType("time64(ns)")

    # A layout is no resolution, and a time of day has nowhere to spell a zone.
    with pytest.raises(ValueError, match="temporal resolution"):
        DataType.time("year_month")
    with pytest.raises(ValueError, match="expected"):
        DataType('time64(us,"UTC")')
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // `time` picks the width from the unit; the leaves name it.
    assert.equal(DataType.time('ms').toString(), 'time32(ms)')
    assert.equal(DataType.time('ns').toString(), 'time64(ns)')
    assert.ok(DataType.from('time').equals(DataType.from('time64(us)')))
    assert.ok(DataType.from('time(3)').equals(DataType.from('time32(ms)')))
    assert.equal(new DataType('time64(us)').id, 'time64')
    assert.equal(new DataType('time32(s)').kind, 'temporal')

    // A layout is no resolution, a unit is required, and a zone has no place.
    assert.throws(() => DataType.time('year_month'), /temporal resolution/)
    assert.throws(() => new DataType('time32(d)'), /expected/)
    assert.throws(() => new DataType('time64(us,"UTC")'), /expected/)
    ```

## Field

`TimeField` is the typed marker, carrying `TimeType` itself so the resolution
and the width are read off the payload. The bindings have a factory per leaf
plus `time(name, unit)`, which picks the width the same way `DataType.time`
does.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, TimeUnit};
    use yggdryl::{TimeField, TimeType};

    let open = TimeField::try_new("open", DataType::time32(TimeUnit::Second)?, false)?;
    assert_eq!(*open.typed_dtype_ref(), TimeType::Time32(TimeUnit::Second));
    assert_eq!(open.typed_dtype_ref().bit_width(), 32);
    assert_eq!(open.dtype(), &DataType::time32(TimeUnit::Second)?);
    assert_eq!(open.id(), DataTypeId::Time32);
    assert!(!open.is_nullable());

    // Another family is refused by the marker, not discovered later.
    assert!(TimeField::try_new("open", DataType::duration32(TimeUnit::Second)?, false).is_err());

    // The payload constructor takes the leaf directly.
    let precise = TimeField::new("precise", TimeType::Time64(TimeUnit::Nanosecond), true);
    assert_eq!(precise.dtype(), &DataType::time64(TimeUnit::Nanosecond)?);
    assert!(precise.is_nullable());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType

    # A factory per leaf, and one that picks the width from the unit.
    assert yggdryl.time32("open", "s").dtype == DataType("time32(s)")
    assert yggdryl.time64("precise", "ns").dtype == DataType("time64(ns)")
    assert yggdryl.time("coarse", "ms").dtype == DataType("time32(ms)")
    assert yggdryl.time("precise", "us").dtype == DataType("time64(us)")

    # Defaults: `time32` is milliseconds, `time64` microseconds.
    assert yggdryl.time32("clock").dtype == DataType("time32(ms)")
    assert yggdryl.time64("clock").dtype == DataType("time64(us)")
    assert yggdryl.time32("clock", nullable=False).nullable is False
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    // A factory per leaf, and one that picks the width from the unit.
    assert.equal(fields.time32('open', 's').dtype.toString(), 'time32(s)')
    assert.equal(fields.time64('precise', 'ns').dtype.toString(), 'time64(ns)')
    assert.equal(fields.time('coarse', 'ms').dtype.toString(), 'time32(ms)')
    assert.equal(fields.time('precise', 'ns').dtype.toString(), 'time64(ns)')

    // Defaults: `time32` is milliseconds, `time64` microseconds.
    assert.ok(fields.time32('clock').dtype.equals(DataType.from('time32(ms)')))
    assert.ok(fields.time64('clock').dtype.equals(DataType.from('time64(us)')))
    assert.equal(fields.time32('clock', 's', { nullable: false }).nullable, false)
    ```

## Scalar

`Scalar::Time32` counts seconds or milliseconds since midnight and
`Scalar::Time64` microseconds or nanoseconds; both carry `Timezone::NAIVE`,
because a time of day names no instant. `Scalar::from_time(count, unit, zone)`
picks the width from the unit.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    // The family constructor picks the width from the unit.
    let noon = Scalar::from_time(43_200, TimeUnit::Second, Timezone::NAIVE)?;
    assert_eq!(noon.as_time32().map(|(count, ..)| count), Some(43_200));
    assert!(matches!(
        Scalar::from_time(1, TimeUnit::Nanosecond, Timezone::NAIVE)?,
        Scalar::Time64(_)
    ));

    // A width refuses a resolution it does not carry, and neither takes a zone.
    assert!(Scalar::time32(1, TimeUnit::Microsecond, Timezone::NAIVE).is_err());
    assert!(Scalar::time64(1, TimeUnit::Millisecond, Timezone::NAIVE).is_err());
    let refused = Scalar::time32(1, TimeUnit::Second, Timezone::UTC).unwrap_err().to_string();
    assert!(refused.contains("timezone"), "{refused}");
    assert!(Scalar::from_time(1, TimeUnit::Day, Timezone::NAIVE).is_err());

    // The shared readers answer across both widths.
    assert_eq!(noon.temporal_unit(), Some(TimeUnit::Second));
    assert_eq!(noon.temporal_timezone(), Some(Timezone::NAIVE));
    assert_eq!(noon.temporal_count_at(TimeUnit::Millisecond), Some(43_200_000));
    assert_eq!(noon.temporal_dtype(), Some(DataType::time32(TimeUnit::Second)?));
    ```

=== "Python"

    ```python
    import datetime as dt

    import pytest

    from yggdryl import DataType, Scalar

    assert DataType("time32(s)").scalar(1).kind == "time32"
    assert DataType("time64(us)").scalar(1).as_py() == dt.time(microsecond=1)
    assert Scalar.from_(dt.time(23, 59, 59, 999_999)).kind == "time64"

    # A time of day carries the zone-free marker and nothing else.
    value = DataType("time32(s)").scalar(1)
    assert (value.count, value.unit, value.zone) == (1, "s", "NAIVE")
    with pytest.raises(ValueError, match="timezone"):
        Scalar.from_(dt.time(1, 2, tzinfo=dt.timezone.utc))

    # The count is checked against its unit's range where the width is declared.
    with pytest.raises(ValueError, match="must be in 0..86400"):
        DataType("time32(s)").scalar(99_999_999)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // JavaScript has no time-of-day value, so a time reads back as the Scalar
    // itself and `count`, `unit` and `zone` are the readers.
    const open = new DataType('time32(s)').scalar(7)
    assert.equal(open.kind, 'time32')
    assert.equal(open.count, 7n)
    assert.equal(open.unit, 's')
    assert.equal(open.zone, 'NAIVE')

    const precise = new DataType('time64(us)').scalar(7n)
    assert.equal(precise.kind, 'time64')
    assert.equal(precise.unit, 'us')
    ```

## Arrow storage

| datatype | Arrow | imports back as |
| --- | --- | --- |
| `time32(s)`, `time32(ms)` | `Time32(unit)` | `time32(unit)` |
| `time64(us)`, `time64(ns)` | `Time64(unit)` | `time64(unit)` |

Both round-trip, so a clock column crosses a boundary as the width and
resolution it declared. Arrow has no zone on a `Time32` or a `Time64` either,
which is the same rule stated once more.

## Text

Text is `HH:MM:SS[.fraction]`, the fraction at the unit's full width - three
digits are milliseconds, six microseconds - so the digits *are* the unit. The
compact `HHMMSS[.fraction]` reads too. An hour past the day folds into it, and
a zone is refused: a time of day is naive, and a zoned reading belongs in a
[datetime](datetime.md).

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    let clock = DataType::time32(TimeUnit::Millisecond)?;
    assert_eq!(
        clock.scalar("10:15:30.5")?,
        Scalar::time32(36_930_500, TimeUnit::Millisecond, Timezone::NAIVE)?
    );

    // The compact spelling is the same reading.
    let nanos = DataType::time64(TimeUnit::Nanosecond)?;
    assert_eq!(nanos.scalar("101530.5")?, nanos.scalar("10:15:30.5")?);

    // An hour past the day folds into it.
    let seconds = DataType::time32(TimeUnit::Second)?;
    assert_eq!(seconds.scalar("25:30:00")?, seconds.scalar("01:30:00")?);

    // A zone makes it an instant, and the refusal says which type to use.
    let refused = seconds.scalar("10:15:30Z").unwrap_err().to_string();
    assert!(refused.contains("DateTime64"), "{refused}");
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl import DataType

    clock = DataType("time32(ms)")
    assert clock.scalar("10:15:30.5").count == 36_930_500

    # The compact spelling is the same reading.
    assert clock.scalar("101530.5") == clock.scalar("10:15:30.5")

    # An hour past the day folds into it.
    seconds = DataType("time32(s)")
    assert seconds.scalar("25:30:00") == seconds.scalar("01:30:00")

    # A zone makes it an instant, and the refusal says which type to use.
    with pytest.raises(ValueError, match="DateTime64"):
        seconds.scalar("10:15:30Z")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    const clock = new DataType('time32(ms)')
    assert.equal(clock.scalar('10:15:30.5').count, 36_930_500n)

    // The compact spelling is the same reading.
    assert.ok(clock.scalar('101530.5').equals(clock.scalar('10:15:30.5')))

    // An hour past the day folds into it.
    const seconds = new DataType('time32(s)')
    assert.ok(seconds.scalar('25:30:00').equals(seconds.scalar('01:30:00')))

    // A zone makes it an instant, and the refusal says which type to use.
    assert.throws(() => seconds.scalar('10:15:30Z'), /DateTime64/)
    ```

## A hand-built leaf

The leaf is public with its unit beside it, so a width can be built around a
resolution it does not carry. `validate` and the Arrow projection are where
that stops, under the width's own name.

```rust
use yggdryl::{DataType, TimeType, TimeUnit};

let broken = DataType::Time32(TimeUnit::Nanosecond);
assert!(broken.validate().is_err());
assert!(broken.clone().into_arrow_datatype().is_err());
assert!(broken.to_arrow_datatype().is_err());

// The constructor and `time_of` refuse the same leaf, with the same reason.
assert!(DataType::time32(TimeUnit::Nanosecond).is_err());
assert!(DataType::time_of(TimeType::Time32(TimeUnit::Nanosecond)).is_err());
assert!(TimeType::Time32(TimeUnit::Nanosecond).validate().is_err());
```

## Edges

- `time32(ns)` -> `unit must be second or millisecond`; `time64(s)` -> `unit must be microsecond or nanosecond`, stated under the width's name.
- `time(d)`, `time(year_month)`, `time(day_time)`, `time(month_day_nano)` -> `unit must be a temporal resolution`, stated under `Time` because no width was chosen yet.
- `time("fortnight")` -> `unknown temporal resolution`; `time(10)` is no precision; Python `DataType.time(1)` -> `TypeError`; JavaScript `DataType.time()` throws.
- `time32` and `time64` require their unit: the grammar has no bare spelling of either, only the family's `time`.
- A count outside its day -> `time-of-day value must be in 0..86400 for s` and the same bound at each finer unit.
- A zone in the text -> refused, naming `DateTime64`; a zone in the spelling -> refused by the grammar, which has nowhere to put one.
- `25:30:00` folds to `01:30:00`; the same text is the next day at `01:30` under a [datetime](datetime.md) column and twenty-five and a half hours under a [duration](duration.md) one.
- [Merged](../field.md) with the other width -> the unit that picks its width; merged with a [duration](duration.md) -> refused, two families.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test root -- temporal::fields temporal::scalars time::temporal
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^time/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_datatype.py -k "time_infers"
    python/.venv/bin/python -m pytest python/tests/test_scalar.py -k "zoned_times_are_refused or finer_than_python"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="generic time" node/tests/datatype.test.js
    node --test --test-name-pattern="typed factory parameters" node/tests/fields.test.js
    ```
