# Duration

An elapsed count at one fixed-length resolution, in one of two widths.

## Contract

| | |
| --- | --- |
| Owned | `DurationType` with its two leaves, `DataType::Duration(DurationType)`, the `DurationField` marker, and the `Duration32` and `Duration64` values |
| Validated | once, at construction: both widths carry the five fixed-length units - `d`, `s`, `ms`, `us`, `ns` - and an interval layout is refused under the width's name. A leaf built by hand is caught by `validate` and by the Arrow projection |
| Lazy | nothing; the leaf is `Copy` and the value is a count, a unit and a zone |
| Cached | the [field](../field.md)'s Arrow projection; the datatype caches nothing |
| Refused | an interval layout, any zone - an elapsed length names no instant - a 32-bit count that does not fit, and a day count at the Arrow boundary |

## DataType

Both widths carry every fixed-length unit, a day included: a day is a length
where a clock has none. The unit is the parameter and the width is the leaf's
own choice, so a column of short spans costs half the bytes and still crosses
Arrow as the 64-bit storage.

| leaf | `DataTypeId` | `as_u8()` | holds | unit | Arrow |
| --- | --- | --- | --- | --- | --- |
| `duration32(unit)` | `Duration32` | `0x36` | `i32` elapsed | `d`, `s`, `ms`, `us`, `ns` | `Duration(unit)`; `d` has no Arrow storage |
| `duration64(unit)` | `Duration64` | `0x37` | `i64` elapsed | `d`, `s`, `ms`, `us`, `ns` | `Duration(unit)`; `d` has no Arrow storage |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `duration32(ms)` | `Duration(DurationType::Duration32(Millisecond))`; the unit is required | - |
| `duration64(ns)` | `Duration(DurationType::Duration64(Nanosecond))`; the unit is required | `Duration(ns)`, Arrow's `Debug` form, capital `D` only |

A bare lowercase `duration` names no width and stays unknown rather than
becoming a width-ambiguous alias; `duration(ms)` is refused for the same
reason. Arrow's own `Duration(unit)` reads as the 64-bit width, because that is
the only one Arrow has.

=== "Rust"

    ```rust
    use yggdryl::DurationType;
    use yggdryl::{DataType, DataTypeId, DataTypeKind, TimeUnit};

    // Two widths, one parameter each; every fixed-length unit is valid at both.
    assert_eq!(DataType::duration32(TimeUnit::Day)?.to_string(), "duration32(d)");
    assert_eq!(DataType::duration64(TimeUnit::Nanosecond)?.to_string(), "duration64(ns)");
    assert_eq!(
        DataType::duration_of(DurationType::Duration64(TimeUnit::Microsecond))?,
        DataType::duration64(TimeUnit::Microsecond)?
    );
    assert_eq!(
        DurationType::ALL,
        [DurationType::Duration32(TimeUnit::Millisecond), DurationType::Duration64(TimeUnit::Microsecond)]
    );

    // The leaf answers its own width, family and identifier.
    let leaf = DataType::duration32(TimeUnit::Second)?.duration_type().expect("a duration datatype");
    assert_eq!(leaf, DurationType::Duration32(TimeUnit::Second));
    assert_eq!(leaf.unit(), TimeUnit::Second);
    assert_eq!(leaf.bit_width(), 32);
    assert_eq!(leaf.family(), "duration");
    assert_eq!(leaf.id(), DataTypeId::Duration32);
    assert_eq!(DataTypeId::Duration32.as_u8(), 0x36);
    assert_eq!(DataType::duration64(TimeUnit::Second)?.kind(), DataTypeKind::Temporal);

    // The grammar reads Arrow's debug form as the width Arrow has.
    assert_eq!(DataType::from_str("Duration(ms)")?, DataType::duration64(TimeUnit::Millisecond)?);
    assert!(DataType::from_str("duration(ms)").is_err());
    assert!(DataType::from_str("duration").is_err());

    // An interval layout is no length at all.
    assert!(DataType::duration64(TimeUnit::MonthDayNano).is_err());
    assert!(DataType::duration32(TimeUnit::YearMonth).is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType

    # Two widths, one parameter each; the factories take the unit.
    assert DataType.duration32("ms") == DataType("duration32(ms)")
    assert DataType.duration64("ns") == DataType("duration64(ns)")
    assert DataType("duration32(ms)").id == "duration32"
    assert DataType("duration64(us)").kind == "temporal"

    # Arrow has one duration width, so a duration imports as `duration64`.
    assert DataType("duration32(ms)").into_arrow() == pa.duration("ms")
    assert DataType.from_arrow(pa.duration("ns")) == DataType("duration64(ns)")

    # An elapsed length has no zone, so the type has nowhere to spell one.
    with pytest.raises(ValueError, match="expected"):
        DataType('duration32(us,"UTC")')
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // JavaScript spells a duration datatype through the grammar; there is no
    // `DataType.duration32` static here.
    assert.equal(new DataType('duration32(ms)').id, 'duration32')
    assert.equal(new DataType('duration64(ns)').toString(), 'duration64(ns)')
    assert.equal(new DataType('duration64(us)').kind, 'temporal')
    assert.ok(DataType.from('Duration(ms)').equals(DataType.from('duration64(ms)')))

    // The unit is required, and a zone has no place in it.
    assert.throws(() => new DataType('duration32'))
    assert.throws(() => new DataType('duration32(s,"UTC")'), /expected/)
    ```

## Field

`DurationField` is the typed marker, carrying `DurationType` itself. The
bindings have one factory per width - `duration32` defaulting to milliseconds
and `duration64` to microseconds - and no width-free `duration` factory,
because a field has to name the width it stores.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, TimeUnit};
    use yggdryl::{DurationField, DurationType};

    let elapsed = DurationField::try_new(
        "elapsed",
        DataType::duration64(TimeUnit::Millisecond)?,
        false,
    )?;
    assert_eq!(*elapsed.typed_dtype_ref(), DurationType::Duration64(TimeUnit::Millisecond));
    assert_eq!(elapsed.typed_dtype_ref().unit(), TimeUnit::Millisecond);
    assert_eq!(elapsed.id(), DataTypeId::Duration64);

    // Another family is refused by the marker.
    assert!(DurationField::try_new("elapsed", DataType::time32(TimeUnit::Second)?, false).is_err());

    // The payload constructor takes the leaf directly.
    let short = DurationField::new("short", DurationType::Duration32(TimeUnit::Second), true);
    assert_eq!(short.dtype(), &DataType::duration32(TimeUnit::Second)?);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, types

    assert types.duration32("elapsed", "ms").dtype == DataType("duration32(ms)")
    assert types.duration64("elapsed", "us").dtype == DataType("duration64(us)")

    # Defaults: `duration32` is milliseconds, `duration64` microseconds.
    assert types.duration32("elapsed").dtype == DataType("duration32(ms)")
    assert types.duration64("elapsed").dtype == DataType("duration64(us)")
    assert types.duration64("elapsed", "ns", nullable=False).nullable is False
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    assert.equal(fields.duration32('elapsed', 'ms').dtype.toString(), 'duration32(ms)')
    assert.equal(fields.duration64('elapsed', 'ns').dtype.toString(), 'duration64(ns)')

    // Defaults: `duration32` is milliseconds, `duration64` microseconds; there
    // is no width-free factory to pick between them.
    assert.ok(fields.duration32('elapsed').dtype.equals(DataType.from('duration32(ms)')))
    assert.ok(fields.duration64('elapsed').dtype.equals(DataType.from('duration64(us)')))
    assert.equal(fields.duration, undefined)
    ```

## Scalar

`Scalar::Duration32` and `Scalar::Duration64` are counts at a fixed-length
unit, carrying `Timezone::NAIVE`. `Scalar::from_duration(count, unit, zone)`
picks the narrowest width that holds the count; the bindings spell that one
`Scalar.duration(count, unit)`, and it is the one construct the type side
cannot express, because `duration32` and `duration64` are two static choices
there.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    // The family constructor picks the narrowest width that holds the count.
    let short = Scalar::from_duration(90, TimeUnit::Second, Timezone::NAIVE)?;
    assert!(short.as_duration32().is_some());
    let long = Scalar::from_duration(i64::from(i32::MAX) + 1, TimeUnit::Second, Timezone::NAIVE)?;
    assert!(long.as_duration64().is_some());

    // The explicit widths take no zone at all, and the `_in` forms refuse one.
    assert_eq!(Scalar::duration32(90, TimeUnit::Second)?.temporal_count(), Some(90));
    assert!(Scalar::duration32_in(1, TimeUnit::Second, Timezone::UTC).is_err());
    assert!(Scalar::duration64_in(1, TimeUnit::Second, Timezone::UTC).is_err());
    assert!(Scalar::duration32(1, TimeUnit::DayTime).is_err());

    // The shared readers answer across both widths.
    assert_eq!(short.temporal_unit(), Some(TimeUnit::Second));
    assert_eq!(short.temporal_timezone(), Some(Timezone::NAIVE));
    assert_eq!(short.temporal_count_at(TimeUnit::Millisecond), Some(90_000));
    assert_eq!(short.temporal_dtype(), Some(DataType::duration32(TimeUnit::Second)?));
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl import Scalar

    # One factory, the width picked from the count.
    assert Scalar.duration(1, "ms").kind == "duration32"
    assert Scalar.duration(2**31, "us").kind == "duration64"
    assert Scalar.duration(1, "us").as_py() == dt.timedelta(microseconds=1)

    # A `timedelta` crosses as itself.
    span = dt.timedelta(days=-2, seconds=3, microseconds=4)
    assert Scalar.from_(span).kind == "duration64"
    assert Scalar.from_(span).as_py() == span

    # An elapsed length carries the zone-free marker and nothing else.
    value = Scalar.duration(1, "ms")
    assert (value.count, value.unit, value.zone) == (1, "ms", "NAIVE")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar } = require('yggdryl')

    // One factory, the width picked from the count.
    assert.equal(Scalar.duration(7, 'ms').kind, 'duration32')
    assert.equal(Scalar.duration(2147483648n, 'us').kind, 'duration64')
    assert.equal(Scalar.duration(7, 'ms').count, 7n)
    assert.equal(Scalar.duration(7, 'ms').zone, 'NAIVE')

    // One length in two resolutions is one value, because the core compares
    // what a value names.
    assert.ok(Scalar.duration(1, 's').equals(Scalar.duration(1000n, 'ms')))
    ```

## Arrow storage

| datatype | Arrow | imports back as |
| --- | --- | --- |
| `duration32(unit)` | `Duration(unit)` | `duration64(unit)` |
| `duration64(unit)` | `Duration(unit)` | `duration64(unit)` |

Arrow stores every duration in 64 bits, so the 32-bit width widens on the way
out and never comes back: a schema that leaves the crate as `duration32(s)`
returns as `duration64(s)`. A day is a length this crate holds and Arrow does
not, so `duration32(d)` and `duration64(d)` are datatypes with no Arrow
storage and the projection refuses them.

```rust
use yggdryl::{DataType, TimeUnit};

// Both widths project to the one Arrow storage.
assert_eq!(
    DataType::duration32(TimeUnit::Second)?.into_arrow_datatype()?,
    DataType::duration64(TimeUnit::Second)?.into_arrow_datatype()?
);

// And the import back is always the 64-bit width.
assert_eq!(
    DataType::from_arrow_datatype(&DataType::duration32(TimeUnit::Second)?.into_arrow_datatype()?)?,
    DataType::duration64(TimeUnit::Second)?
);

// A day count is a duration, but not one Arrow can store.
assert!(DataType::duration64(TimeUnit::Day)?.into_arrow_datatype().is_err());
```

## Text

Text is the ISO duration `PT<seconds>[.fraction]S`, the sign leading: seconds
are the one component every unit restates exactly, so the writer never
decomposes into hours a reader would multiply back. Reading takes the general
form `P1DT2H3M4.5S` and a plain clock `-25:30:00.5`, whose hours never fold -
that is a [time of day](time.md)'s rule, not a length's.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit};

    let seconds = DataType::duration64(TimeUnit::Second)?;
    assert_eq!(seconds.scalar("P1DT2H3M4S")?, Scalar::duration64(93_784, TimeUnit::Second)?);
    assert_eq!(seconds.scalar("-25:30:00")?, Scalar::duration64(-91_800, TimeUnit::Second)?);

    // The writer states seconds, whichever width held the count.
    assert_eq!(
        DataType::utf8().scalar(Scalar::duration32(90, TimeUnit::Second)?)?,
        Scalar::from("PT90S")
    );
    assert_eq!(
        DataType::utf8().scalar(Scalar::duration64(90, TimeUnit::Second)?)?,
        Scalar::from("PT90S")
    );

    // ISO 8601 names the comma the preferred decimal sign, so both read alike.
    let millis = DataType::duration64(TimeUnit::Millisecond)?;
    assert_eq!(millis.scalar("PT1,5S")?, millis.scalar("PT1.5S")?);
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    seconds = DataType("duration64(s)")
    assert seconds.scalar("P1DT2H3M4S") == Scalar.duration(93_784, "s")
    assert seconds.scalar("-25:30:00") == Scalar.duration(-91_800, "s")

    # The writer states seconds, whichever width held the count.
    assert DataType("utf8").scalar(Scalar.duration(90, "s")).as_py() == "PT90S"

    # ISO 8601 names the comma the preferred decimal sign, so both read alike.
    millis = DataType("duration64(ms)")
    assert millis.scalar("PT1,5S") == millis.scalar("PT1.5S")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    const seconds = new DataType('duration64(s)')
    assert.ok(seconds.scalar('P1DT2H3M4S').equals(Scalar.duration(93_784n, 's')))
    assert.ok(seconds.scalar('-25:30:00').equals(Scalar.duration(-91_800n, 's')))

    // The writer states seconds, whichever width held the count.
    assert.equal(new DataType('utf8').scalar(Scalar.duration(90, 's')).asStr(), 'PT90S')

    // ISO 8601 names the comma the preferred decimal sign, so both read alike.
    const millis = new DataType('duration64(ms)')
    assert.ok(millis.scalar('PT1,5S').equals(millis.scalar('PT1.5S')))
    ```

## Arithmetic

A duration is the difference between two instants and the step that moves one,
so arithmetic across the families answers a duration or an instant rather than
a bare number. An [interval](interval.md) takes none of it: a month restates no
count of days.

=== "Rust"

    ```rust
    use yggdryl::{Scalar, TimeUnit, Timezone};

    let at = Scalar::datetime64(1_000, TimeUnit::Millisecond, Timezone::UTC)?;
    let elapsed = Scalar::duration64(2, TimeUnit::Second)?;

    // An instant plus a length is an instant, at the instant's own resolution.
    assert_eq!(
        at.checked_add(&elapsed)?,
        Scalar::datetime64(3_000, TimeUnit::Millisecond, Timezone::UTC)?
    );

    // Two instants differ by a length, and the zone is preserved.
    assert_eq!(
        at.checked_sub(&Scalar::datetime64(500, TimeUnit::Millisecond, Timezone::UTC)?)?,
        Scalar::duration64(500, TimeUnit::Millisecond)?
    );

    // Two days differ by a length too, at the coarser unit that is exact.
    assert_eq!(
        Scalar::date32(2).checked_sub(&Scalar::date32(1))?,
        Scalar::duration64(1, TimeUnit::Day)?
    );
    ```

=== "Python"

    ```python
    from yggdryl import DataType, Scalar

    at = DataType('datetime64(ms,"UTC")').scalar(1_000)

    # An instant plus a length is an instant, at the instant's own resolution.
    later = at + Scalar.duration(2, "s")
    assert later.kind == "datetime64"
    assert (later.count, later.unit, later.zone) == (3_000, "ms", "UTC")

    # Two instants differ by a length.
    elapsed = at - DataType('datetime64(ms,"UTC")').scalar(500)
    assert elapsed.family == "temporal"
    assert (elapsed.count, elapsed.unit) == (500, "ms")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    const at = new DataType('datetime64(ms,"UTC")').scalar(1000n)
    assert.ok(
      at.add(Scalar.duration(2n, 's')).equals(new DataType('datetime64(ms,"UTC")').scalar(3000n)),
    )
    assert.ok(
      at.subtract(new DataType('datetime64(ms,"UTC")').scalar(500n))
        .equals(Scalar.duration(500n, 'ms')),
    )
    ```

## Edges

- `duration32(year_month)`, `duration64(day_time)` -> `unit must be day, second, millisecond, microsecond, or nanosecond`, stated under the width's name by the constructor, `duration_of`, `validate` and the root alike.
- `duration64(d)` is a datatype and has no Arrow storage, so the projection refuses it; its canonical spelling `duration32(d)` renders but the grammar does not read it back.
- Lowercase `duration` and `duration(ms)` -> unknown: a bare word names no width, and a width-ambiguous alias is not offered.
- A zone -> refused at the datatype, at `duration32_in`/`duration64_in`, and in the text: an elapsed length names no instant.
- A 32-bit count that does not fit -> refused by `Scalar::duration32`, widened by `Scalar::from_duration`, which is the only place that widens.
- Arrow `Duration(unit)` -> `duration64(unit)`; the 32-bit width widens on the way out and never comes back.
- `-25:30:00` is twenty-five and a half hours, not `01:30:00`: a length has no day to fold into.
- [Merged](../field.md) with the other width -> `duration64`; merged with a [time](time.md) or an [interval](interval.md) -> refused, two families.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::temporal::every_duration_leaf field::temporal::a_duration_field temporal::family_constructors
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --lib -- arithmetic::tests::temporal_arithmetic arithmetic::tests::durations_scale
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^duration/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_native_scalar.py -k "width_unit_scale_and_zone or exact_width_factories"
    python/.venv/bin/python -m pytest python/tests/types/test_scalar.py -k "temporals_cross"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="temporal families|Scalar family factories" node/tests/text/codec.test.js
    node --test --test-name-pattern="typed field factories" node/tests/types/fields.test.js
    ```
