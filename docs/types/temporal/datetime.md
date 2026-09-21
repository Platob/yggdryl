# Datetime

An instant or a wall-clock reading: one leaf, `datetime64`, carrying its resolution and its zone.

## Contract

| | |
| --- | --- |
| Owned | `DateTimeType` with its single `DateTime64 { unit, timezone }` leaf, `DataType::DateTime(DateTimeType)`, the `DateTimeField` marker, and the `DateTime64` value |
| Validated | once, at construction: the unit must be a clock resolution - seconds, milliseconds, microseconds or nanoseconds - and the zone is canonicalized on arrival. `with_unit` restates the resolution under the same rule; `with_timezone` cannot fail, because every zone is valid at every resolution |
| Lazy | nothing; the leaf is `Copy` and the value is a count, a unit and a zone |
| Cached | the [field](../field.md)'s Arrow projection; the datatype caches nothing |
| Refused | a day or an interval layout as the resolution, a zone that names no zone, a zoned column reading text with no offset, and a naive column reading text that carries one |

## DataType

One leaf with two parameters. [`Timezone::NAIVE`](timezone.md) is a wall clock
and projects to Arrow as a timestamp with no zone; any other zone makes the
count a UTC instant and rides as the zone's canonical name.

| leaf | `DataTypeId` | `as_u8()` | holds | unit | Arrow |
| --- | --- | --- | --- | --- | --- |
| `datetime64(unit)` | `DateTime64` | `0x31` | `i64` since the Unix epoch, a wall clock | `s`, `ms`, `us`, `ns` | `Timestamp(unit, None)` |
| `datetime64(unit,"zone")` | `DateTime64` | `0x31` | `i64` since the Unix epoch, UTC | `s`, `ms`, `us`, `ns` | `Timestamp(unit, Some(zone))` |

| spelling | datatype | also parsed as |
| --- | --- | --- |
| `datetime64(us)` | naive, microseconds; both parameters default | `datetime64`, `timestamp`, `timestamp_ntz`, `timestamp(6)`, `timestamp without time zone`, `datetime64(us, None)` |
| `datetime64(us,"UTC")` | UTC, microseconds | `datetime64(us, UTC)`, `timestamp_ltz`, `timestamp with time zone`, `datetime64(us, Some(UTC))`, `datetime64(us, timezone=UTC)` |
| `datetime64(ns,"Europe/Paris")` | a named zone | `datetime64(9, Europe/Paris)`, any spelling [`Timezone`](timezone.md) canonicalizes |

The zone is quoted in the canonical spelling and may be bare in the grammar; a
precision reads as the unit it needs, as [`time(p)`](time.md) does, and `Some` /
`None` are Arrow's own `Debug` form. FIX's date and timestamp names resolve
here too - `UTCTimestamp` and `TZTimestamp` to `datetime64(ns,"UTC")`,
`LocalMktDate` to `datetime64(ns)` at that day's midnight - and the
[datatype page](../datatype.md) lists the whole logical vocabulary.

=== "Rust"

    ```rust
    use yggdryl::DateTimeType;
    use yggdryl::{DataType, DataTypeId, DataTypeKind, TimeUnit, Timezone};

    // One leaf, two parameters; a naive column and a zoned one are one datatype.
    let naive = DataType::datetime64(TimeUnit::Microsecond, Timezone::NAIVE)?;
    let utc = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?;
    assert_eq!(naive.to_string(), "datetime64(us)");
    assert_eq!(utc.to_string(), "datetime64(us,\"UTC\")");
    assert_eq!(DataTypeId::DateTime64.as_u8(), 0x31);
    assert_eq!(naive.kind(), DataTypeKind::Temporal);

    // The grammar: the crate's spelling, Arrow's, SQL's and Spark's.
    assert_eq!(DataType::from_str("timestamp")?, naive);
    assert_eq!(DataType::from_str("datetime64")?, naive);
    assert_eq!(DataType::from_str("timestamp_ntz")?, naive);
    assert_eq!(DataType::from_str("timestamp(6)")?, naive);
    assert_eq!(DataType::from_str("timestamp without time zone")?, naive);
    assert_eq!(DataType::from_str("timestamp_ltz")?, utc);
    assert_eq!(DataType::from_str("datetime64(us, UTC)")?, utc);
    assert_eq!(DataType::from_str("timestamp(9) with time zone")?.to_string(), "datetime64(ns,\"UTC\")");

    // The leaf reads both parameters back, and restates one at a time.
    let leaf = naive.datetime_type().expect("a datetime datatype");
    assert_eq!(leaf, DateTimeType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE });
    assert_eq!(leaf.unit(), TimeUnit::Microsecond);
    assert_eq!(leaf.bit_width(), 64);
    assert_eq!(leaf.family(), "datetime");
    assert_eq!(leaf.with_timezone(Timezone::UTC).timezone(), Timezone::UTC);
    assert_eq!(leaf.with_unit(TimeUnit::Second)?.unit(), TimeUnit::Second);
    assert_eq!(DateTimeType::ALL, [leaf]);

    // A day is no resolution of a clock, wherever the rule is checked.
    assert!(leaf.with_unit(TimeUnit::Day).is_err());
    assert!(DataType::datetime64(TimeUnit::YearMonth, Timezone::UTC).is_err());
    assert!(DataType::DateTime(DateTimeType::DateTime64 {
        unit: TimeUnit::Day,
        timezone: Timezone::NAIVE,
    })
    .validate()
    .is_err());
    ```

=== "Python"

    ```python
    import pyarrow as pa
    import pytest

    from yggdryl import DataType

    # One leaf, two parameters, every spelling of it.
    assert DataType("timestamp") == DataType("datetime64(us)")
    assert DataType("timestamp_ntz") == DataType("datetime64")
    assert DataType("timestamp_ltz") == DataType('datetime64(us,"UTC")')
    assert DataType("datetime64(ns, UTC)") == DataType('datetime64(ns,"UTC")')
    assert str(DataType("timestamp(6)")) == "datetime64(us)"
    assert DataType("datetime64(ns)").id == "datetime64"
    assert DataType("datetime64(ns)").kind == "temporal"

    # The zone reads back as the value a temporal column declares.
    assert str(DataType('datetime64(us,"Europe/Paris")').timezone) == "Europe/Paris"
    assert str(DataType("datetime64(us)").timezone) == "NAIVE"

    # Arrow carries the zone only when the datatype states one.
    assert DataType('datetime64(us,"UTC")').into_arrow() == pa.timestamp("us", tz="UTC")
    assert DataType("datetime64(us)").into_arrow() == pa.timestamp("us")
    assert DataType.from_arrow(pa.timestamp("ns", tz="UTC")) == DataType('datetime64(ns,"UTC")')

    with pytest.raises(ValueError, match="temporal resolution"):
        DataType("datetime64(year_month)")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType } = require('yggdryl')

    // One leaf, two parameters, every spelling of it.
    assert.ok(DataType.from('timestamp').equals(DataType.from('datetime64(us)')))
    assert.ok(DataType.from('timestamp_ltz').equals(DataType.from('datetime64(us,"UTC")')))
    assert.ok(DataType.from('datetime64(us,UTC)').equals(DataType.from('datetime64(us,"UTC")')))
    assert.equal(DataType.from('timestamp(6)').toString(), 'datetime64(us)')
    assert.equal(new DataType('datetime64(ns)').id, 'datetime64')
    assert.equal(new DataType('datetime64(ns)').kind, 'temporal')

    // A naive instant is not the same datatype as a zoned one.
    assert.ok(!DataType.from('datetime64(ns)').equals(DataType.from('datetime64(ns,"UTC")')))
    assert.throws(() => new DataType('datetime64(year_month)'))
    ```

## Field

`DateTimeField` is the typed marker, carrying `DateTimeType` itself, so the
resolution and the zone are read off the payload rather than reparsed. The
bindings have one factory taking both parameters, the zone defaulting to
`NAIVE`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, DataTypeId, TimeUnit, Timezone};
    use yggdryl::{DateTimeField, DateTimeType};

    let dtype = DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC)?;
    let transact = DateTimeField::try_new("transact", dtype.clone(), false)?;
    assert_eq!(transact.typed_dtype_ref().unit(), TimeUnit::Microsecond);
    assert!(transact.typed_dtype_ref().timezone().is_utc());
    assert_eq!(transact.dtype(), &dtype);
    assert_eq!(transact.id(), DataTypeId::DateTime64);

    // Another family is refused by the marker.
    assert!(DateTimeField::try_new("transact", DataType::date32(), false).is_err());

    // The payload constructor takes the leaf directly.
    let at = DateTimeField::new(
        "at",
        DateTimeType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::NAIVE },
        false,
    );
    assert_eq!(at.dtype().to_string(), "datetime64(us)");
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, types

    at = types.datetime64("at", "us", "UTC", nullable=False)
    assert at.name == "at"
    assert str(at.dtype) == 'datetime64(us,"UTC")'
    assert str(at.dtype.timezone) == "UTC"
    assert at.nullable is False
    assert at.into_arrow().type == pa.timestamp("us", tz="UTC")

    # Both parameters default: microseconds, and the zone-free marker.
    assert types.datetime64("at").dtype == DataType("datetime64(us)")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields } = require('yggdryl')

    const at = fields.datetime64('at', 'us', 'UTC', { nullable: false })
    assert.equal(at.name, 'at')
    assert.equal(at.dtype.toString(), 'datetime64(us,"UTC")')
    assert.equal(at.dtype.id, 'datetime64')
    assert.equal(at.nullable, false)

    // Both parameters default; omitting the zone makes the third argument the
    // options object.
    assert.ok(fields.datetime64('at').dtype.equals(DataType.from('datetime64(us)')))
    assert.equal(fields.datetime64('at', 'ms', { nullable: false }).nullable, false)
    ```

## Scalar

`Scalar::DateTime64` is a 64-bit count carrying its unit and its zone. The
count is UTC, as Arrow defines it, so a zoned value and its naive twin are two
different values even at the same count.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    let at = Scalar::from_datetime(1, TimeUnit::Microsecond, Timezone::UTC)?;
    assert_eq!(at, Scalar::datetime64(1, TimeUnit::Microsecond, Timezone::UTC)?);
    assert_eq!(at.as_datetime64().map(|(count, unit, _)| (count, unit)), Some((1, TimeUnit::Microsecond)));
    assert_eq!(at.temporal_timezone(), Some(Timezone::UTC));
    assert_eq!(at.temporal_count_at(TimeUnit::Nanosecond), Some(1_000));

    // A zone is named rather than parsed twice.
    assert_eq!(Scalar::datetime64_in(0, TimeUnit::Second, "Asia/Calcutta")?.temporal_timezone(),
        Some(Timezone::from_str("Asia/Kolkata")?));

    // A naive reading is not the zoned one, even at the same count.
    let naive = Scalar::datetime64(1, TimeUnit::Microsecond, Timezone::NAIVE)?;
    assert_ne!(naive, at);
    assert_eq!(naive.temporal_timezone(), Some(Timezone::NAIVE));

    // A day is no resolution of a clock, at the value too.
    assert!(Scalar::from_datetime(1, TimeUnit::Day, Timezone::NAIVE).is_err());
    ```

=== "Python"

    ```python
    import datetime as dt
    import zoneinfo

    from yggdryl import DataType, Scalar

    instant = DataType('datetime64(us,"UTC")').scalar(0)
    assert instant.kind == "datetime64"
    assert (instant.count, instant.unit, instant.zone) == (0, "us", "UTC")
    assert instant.as_py() == dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc)

    # An aware `datetime` crosses as itself, live zone and all.
    paris = zoneinfo.ZoneInfo("Europe/Paris")
    aware = dt.datetime(2026, 8, 15, 12, 3, 4, 5, tzinfo=paris)
    restored = Scalar.from_(aware).as_py()
    assert restored == aware
    assert restored.tzinfo.key == "Europe/Paris"

    # A naive `datetime` crosses as a naive value.
    assert Scalar.from_(dt.datetime(2026, 8, 15, 12, 3, 4, 5)).kind == "datetime64"
    assert Scalar.from_(dt.datetime(2026, 8, 15)).zone == "NAIVE"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, Scalar } = require('yggdryl')

    // A `Date` is the JavaScript spelling of a UTC millisecond datetime64.
    const date = new Date('2026-08-15T12:30:00.000Z')
    assert.ok(Scalar.from(date).equals(new DataType('datetime64(ms,"UTC")').scalar(1786797000000n)))
    assert.ok(Scalar.from(date).asJs() instanceof Date)

    // A resolution or a zone a `Date` cannot hold stays a Scalar, read through
    // `count`, `unit` and `zone`.
    const precise = new DataType('datetime64(ns,"Europe/Paris")').scalar(1700000000123456789n)
    assert.equal(precise.kind, 'datetime64')
    assert.equal(precise.unit, 'ns')
    assert.equal(precise.zone, 'Europe/Paris')

    // A naive instant is not the same value as a zoned one.
    assert.ok(!new DataType('datetime64(ns)').scalar(7n).equals(
      new DataType('datetime64(ns,"UTC")').scalar(7n),
    ))
    ```

## Arrow storage

| datatype | Arrow | imports back as |
| --- | --- | --- |
| `datetime64(unit)` | `Timestamp(unit, None)` | `datetime64(unit)` |
| `datetime64(unit,"zone")` | `Timestamp(unit, Some("zone"))` | `datetime64(unit,"zone")`, the zone canonical |

The zone rides only when the datatype states one, which is exactly how Arrow
spells a wall clock; the zone-free marker is not written as a name. The import
canonicalizes what it finds, so `Asia/Calcutta` in a foreign schema arrives as
`Asia/Kolkata` and compares equal to it.

## Text

A wall clock writes `YYYY-MM-DDTHH:MM:SS[.fraction]`. A zoned instant writes
its local reading, the offset that recovers the instant - `Z` for UTC - and the
zone's name in brackets when the name is a place rather than an offset, because
`+02:00` cannot say `Europe/Paris`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

    // The count is UTC, as Arrow defines it; the text is the local reading.
    let utc = DataType::datetime64(TimeUnit::Second, Timezone::UTC)?;
    assert_eq!(
        utc.scalar("1970-01-01T00:00:10Z")?,
        Scalar::datetime64(10, TimeUnit::Second, Timezone::UTC)?
    );
    assert_eq!(utc.scalar("19700101Z")?, Scalar::datetime64(0, TimeUnit::Second, Timezone::UTC)?);
    assert_eq!(
        DataType::utf8().scalar(utc.scalar("1970-01-01T00:00:10Z")?)?,
        Scalar::from("1970-01-01T00:00:10Z")
    );

    // One instant, every spelling: extended, compact, and FIX's mixture.
    let naive = DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE)?;
    let stated = naive.scalar("2024-01-02T10:15:30")?;
    for held in ["20240102101530", "20240102-10:15:30", "2024-01-02 10:15:30"] {
        assert_eq!(naive.scalar(held)?, stated, "{held}");
    }

    // ISO 8601 names the comma the preferred decimal sign, so both read alike.
    let millis = DataType::datetime64(TimeUnit::Millisecond, Timezone::NAIVE)?;
    assert_eq!(
        millis.scalar("2026-08-14T00:05:01,148")?,
        millis.scalar("2026-08-14T00:05:01.148")?
    );
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl import DataType

    # A count finer than Python holds is floored, not withheld, and the
    # flooring is monotonic across the epoch.
    utc = DataType('datetime64(ns,"UTC")')
    assert utc.scalar(1).as_py() == dt.datetime(1970, 1, 1, tzinfo=dt.timezone.utc)
    assert utc.scalar(1_500).as_py() == dt.datetime(1970, 1, 1, 0, 0, 0, 1, tzinfo=dt.timezone.utc)
    assert utc.scalar(-1).as_py() == dt.datetime(
        1969, 12, 31, 23, 59, 59, 999_999, tzinfo=dt.timezone.utc
    )

    # A coarser unit is restated exactly.
    seconds = DataType('datetime64(s,"UTC")')
    assert seconds.scalar(1_700_000_000).as_py() == dt.datetime(
        2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, json } = require('yggdryl')

    // On the wire every temporal is its classic ISO string.
    assert.equal(
      json.loads(json.dumps(new DataType('datetime64(s)').scalar(1786797000n))),
      '2026-08-15T12:30:00',
    )
    // A place is named in brackets, because an offset cannot say which zone.
    assert.equal(
      json.loads(json.dumps(new DataType('datetime64(ms,"Europe/Paris")').scalar(1786797000000n))),
      '2026-08-15T14:30:00.000+02:00[Europe/Paris]',
    )
    ```

## A date under a datetime column

A reading that states no clock is that day's midnight, which is how a
settlement [date](date.md) sits in the column a transact time sits in rather
than in a second one to cast through. A zoned column still wants the zone
stated, and a naive one refuses one.

```rust
use yggdryl::{DataType, Scalar, TimeUnit, Timezone};

let naive = DataType::datetime64(TimeUnit::Nanosecond, Timezone::NAIVE)?;
let utc = DataType::datetime64(TimeUnit::Nanosecond, Timezone::UTC)?;
let midnight = naive.scalar(Scalar::from("2024-01-02T00:00:00"))?;
for held in ["2024-01-02", "20240102"] {
    assert_eq!(naive.scalar(Scalar::from(held))?, midnight, "{held}");
}

let zoned_midnight = utc.scalar(Scalar::from("2024-01-02T00:00:00Z"))?;
for held in ["2024-01-02Z", "20240102Z", "20240102+00:00"] {
    assert_eq!(utc.scalar(Scalar::from(held))?, zoned_midnight, "{held}");
}
assert!(utc.scalar(Scalar::from("20240102")).is_err());
assert!(naive.scalar(Scalar::from("20240102Z")).is_err());
```

## Edges

- `datetime64(d, tz)`, `datetime64(year_month, tz)` -> `unit must be a temporal resolution`, under the kind `datetime64`; `DataType::validate` on a hand-built leaf says the same.
- A hand-built leaf its unit does not allow fails `validate`, `into_arrow_datatype` and `into_arrow_datatype_ffi` alike; the constructor refuses it first.
- A zoned column reading text with no offset -> refused; a naive column reading text that carries one -> refused. The zone is the column's and the text has to agree.
- `2024010210153`, `202401021015300`, `20241302101530` -> refused at the position that broke, never rounded into a neighbouring reading.
- A fraction with no digits after its decimal sign -> refused; a grouped fraction reads at the width the grouping spells.
- An hour past the day carries into the next date, where a [time of day](time.md) folds into its own.
- `datetime64(s,"Mars/Olympus")` -> the datatype holds, and the Python projection names the zone it has no rules for.
- [Merged](../field.md) -> one unit, and a zone one side declares is kept; merged with a [date](date.md) or a [time](time.md) -> refused, two families.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" --manifest-path rust/Cargo.toml -p yggdryl --test types -- datatype::temporal::the_datetime_leaf datatype::temporal::a_temporal_reads datatype::temporal::either_decimal_sign field::temporal::a_datetime_field
    cargo bench --manifest-path rust/Cargo.toml --bench types -- '^(datetime|temporal_text)/'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/types/test_scalar.py -k "aware_datetime or ambiguous_zoned or finer_than_python or coarser_unit or zone_with_no_rules"
    python/.venv/bin/python -m pytest python/tests/types/test_native_scalar.py -k "width_unit_scale_and_zone"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="temporal families|a Date is the JavaScript spelling" node/tests/text/codec.test.js
    node --test --test-name-pattern="defaulted temporal" node/tests/types/fields.test.js
    ```
