# Temporal — dates, times, timestamps, durations

The `temporal` family models time over two shared axes — a **resolution** (`TimeUnit`,
nanoseconds → years) and a **timezone** (`Tz`: naive, UTC, a fixed offset, or a DST-aware IANA
zone backed by the full tz database) — under a set of self-describing, byte-width value types:

| concept | widths | backing | meaning |
| --- | --- | --- | --- |
| date | `Date32` / `Date64` | i32 days / i64 millis | a calendar day (naive) |
| time of day | `Time32` / `Time64` | i32 (s/ms) / i64 (µs/ns) | a wall-clock time (naive) |
| timestamp | `Ts32` / `Ts64` / `Ts96` | i32 / i64 / 96-bit count since epoch | an instant (naive or zoned) |
| duration | `Duration32` / `Duration64` | i32 / i64 count | an elapsed span |

Every value reports its `time_unit()` and `timezone()` (the shared `Temporal` trait). In the
bindings, **units and timezones cross as strings** — `"ns"`/`"ms"`/`"s"`/`"second"` and
`"UTC"`/`"Europe/Paris"`/`"+02:00"`/`""` (naive) — so the API stays idiomatic.

!!! tip "Portable, high-performance temporal instances for Python & Node"
    These types are meant to be used **as your temporal values**, in place of `datetime` /
    `Date` — the same instant behaves identically in all three languages. You get what the native
    types don't: **DST-correct IANA zones** (the full tz database, offset lookups are
    allocation-free), a range far past the native limits (`Ts96` reaches the year 5000+ at
    nanosecond resolution), an explicit unit so no silent millisecond↔second confusion, exact
    calendar math, value **identity + hashing + pickling/serialization** (usable as dict keys and
    over a wire), and free conversions between every temporal concept
    ([above](#converting-between-the-temporal-types)). Bridge to the platform types only at the
    edges (`from_pydatetime` / `to_pydatetime`, `new Date(ts.toEpochMillis())`).

## Dates — the calendar

Exact proleptic-Gregorian math (leap years, weekdays, negative years), with `Date32 ↔ Date64`.

=== "Python"

    ```python
    from yggdryl.temporal import Date32

    d = Date32.from_ymd(2024, 2, 29)          # a leap day
    assert d.to_ymd() == (2024, 2, 29)
    assert d.weekday() == 4 and d.is_leap_year()   # a Thursday (0=Sunday)
    assert str(d) == "2024-02-29"
    assert Date32.from_days(0).to_ymd() == (1970, 1, 1)   # the epoch
    ```

=== "Node"

    ```js
    const { Date32 } = require('yggdryl').temporal

    const d = Date32.fromYmd(2024, 2, 29)
    assert.deepEqual(d.toYmd(), [2024, 2, 29])
    assert(d.weekday() === 4 && d.isLeapYear())
    assert(d.toString() === '2024-02-29')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::temporal::Date32;

    let d = Date32::from_ymd(2024, 2, 29).unwrap();
    assert_eq!(d.to_ymd(), (2024, 2, 29));
    assert!(d.weekday() == 4 && d.is_leap_year());
    assert_eq!(d.to_string(), "2024-02-29");
    ```

## Timestamps — the instant, zone-aware

A timestamp is a UTC-relative count plus a timezone; its wall-clock **moves with the zone**
(DST-aware, from the IANA database). The stored instant never changes when you re-zone it.

=== "Python"

    ```python
    from yggdryl.temporal import Ts64

    utc = Ts64.from_datetime(2024, 7, 15, 12, 0, 0, 0, "s", "UTC")
    assert utc.to_datetime() == (2024, 7, 15, 12, 0, 0, 0)

    paris = utc.with_timezone("Europe/Paris")             # the SAME instant
    assert paris.to_datetime() == (2024, 7, 15, 14, 0, 0, 0)   # +2h in summer (CEST)
    assert paris.epoch_value == utc.epoch_value
    assert str(paris).endswith("+02:00")

    assert utc.to_date().to_ymd() == (2024, 7, 15)         # extract the date
    assert utc.to_unit("ms").epoch_value == utc.epoch_value * 1000
    ```

=== "Node"

    ```js
    const { Ts64 } = require('yggdryl').temporal

    const utc = Ts64.fromDatetime(2024, 7, 15, 12, 0, 0, 0, 's', 'UTC')
    assert.deepEqual(utc.toDatetime(), [2024, 7, 15, 12, 0, 0, 0])

    const paris = utc.withTimezone('Europe/Paris')       // the SAME instant
    assert.deepEqual(paris.toDatetime(), [2024, 7, 15, 14, 0, 0, 0])   // +2h summer
    assert(paris.epochValue === utc.epochValue)          // bigint
    assert(paris.toString().endsWith('+02:00'))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::temporal::{Ts64, TimeUnit, Tz};

    let utc = Ts64::from_datetime(2024, 7, 15, 12, 0, 0, 0, TimeUnit::Second, Tz::UTC).unwrap();
    let paris = utc.with_timezone(Tz::europe_paris());
    assert_eq!(paris.to_datetime(), (2024, 7, 15, 14, 0, 0, 0)); // +2h (CEST)
    assert_eq!(paris.epoch_value(), utc.epoch_value());          // same instant
    ```

`Ts96` holds a nanosecond count far beyond `i64`'s ~292-year range (e.g. the year 5000),
and `to_ts32/64/96` convert between the widths. Winter reads `+01:00` (CET) for Paris —
the offset comes from the tz database at the instant, so DST is correct.

## Durations — elapsed spans

A signed count in a unit, with unit-aligning arithmetic.

=== "Python"

    ```python
    from yggdryl.temporal import Duration64

    total = Duration64.seconds(1) + Duration64.milliseconds(500)
    assert (total.value, total.unit) == (1500, "ms")     # aligns to the finer unit
    assert str(Duration64.seconds(90)) == "90s"
    assert Duration64.seconds(1) > Duration64.milliseconds(500)   # by elapsed span

    # Flexible string parse — compound, clock, or ISO-8601; natural granularity.
    assert (Duration64.from_string("1h30m").value, Duration64.from_string("1h30m").unit) == (90, "min")
    assert Duration64.from_string("PT1.5S").value == 1500        # ISO-8601 → ms
    ```

=== "Node"

    ```js
    const { Duration64 } = require('yggdryl').temporal

    const total = Duration64.seconds(1).add(Duration64.milliseconds(500))
    assert.deepEqual([total.value, total.unit], [1500, 'ms'])
    assert.equal(Duration64.seconds(90).toString(), '90s')
    assert.equal(Duration64.seconds(1).compareTo(Duration64.milliseconds(500)), 1)

    // Flexible string parse — compound, clock, or ISO-8601; natural granularity.
    assert.deepEqual([Duration64.fromString('1h30m').value, Duration64.fromString('1h30m').unit], [90, 'min'])
    assert.equal(Duration64.fromString('PT1.5S').value, 1500)    // ISO-8601 → ms
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::temporal::{Duration64, TimeUnit};

    let total = Duration64::seconds(1).checked_add(&Duration64::milliseconds(500)).unwrap();
    assert_eq!((total.value(), total.unit()), (1500, TimeUnit::Millisecond));
    assert!(Duration64::seconds(1) > Duration64::milliseconds(500));

    // Flexible string parse — compound, clock, or ISO-8601; natural granularity.
    let d = Duration64::parse_str("1h30m").unwrap();
    assert_eq!((d.value(), d.unit()), (90, TimeUnit::Minute));
    ```

## Converting between the temporal types

Every temporal type converts to every other — a date becomes an instant (`at_midnight` /
`at_time`), an instant yields its date / time / elapsed span (`to_date` · `to_time` ·
`to_duration`), a span becomes an instant / time / date (`to_timestamp` · `to_time` · `to_date`),
and a time lands on the epoch date (`to_timestamp`). Widths convert within a concept too
(`to_date32/64`, `to_time32/64`, `to_ts32/64/96`, `to_duration32/64`).

=== "Python"

    ```python
    from yggdryl.temporal import Date32, Time64

    date = Date32.from_ymd(2024, 2, 29)
    time = Time64.from_hms_nano(13, 45, 30, 0)

    ts = date.at_time(time, "s", "UTC")            # date + time -> instant
    assert ts.to_datetime() == (2024, 2, 29, 13, 45, 30, 0)
    assert ts.to_date() == date                    # instant -> date
    assert ts.to_time().to_hms() == (13, 45, 30, 0)  # instant -> time

    span = date.to_duration()                      # days since the epoch, as a span
    assert span.to_date() == date                  # and back
    assert time.to_duration().to_time().to_hms() == (13, 45, 30, 0)
    ```

=== "Node"

    ```js
    const { Date32, Time64 } = require('yggdryl').temporal

    const date = Date32.fromYmd(2024, 2, 29)
    const time = Time64.fromHmsNano(13, 45, 30, 0)

    const ts = date.atTime(time, 's', 'UTC')       // date + time -> instant
    assert.deepEqual(ts.toDatetime(), [2024, 2, 29, 13, 45, 30, 0])
    assert.ok(ts.toDate().equals(date))            // instant -> date
    assert.deepEqual(ts.toTime().toHms(), [13, 45, 30, 0])

    const span = date.toDuration()                 // days since epoch, as a span
    assert.ok(span.toDate().equals(date))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::temporal::{Date32, Time64, TimeUnit, Tz};

    let date = Date32::from_ymd(2024, 2, 29).unwrap();
    let time = Time64::from_hms_nano(13, 45, 30, 0).unwrap();

    let ts = date.at_time(&time, TimeUnit::Second, Tz::UTC).unwrap();
    assert_eq!(ts.to_datetime(), (2024, 2, 29, 13, 45, 30, 0));
    assert_eq!(ts.to_date().unwrap(), date);            // instant -> date
    assert_eq!(date.to_duration().to_date().unwrap(), date); // via a span
    ```

## The type system knows temporals

Temporals are their own [`DataTypeCategory`](../schema.md) — fixed-width, but **not** numeric — so
`is_temporal()` drills down like every other family, and the schema layer names them.

=== "Python"

    ```python
    from yggdryl.types import DataType

    dt = DataType.ts64()
    assert dt.name == "ts64" and dt.category == "temporal"
    assert dt.is_temporal() and not dt.is_numeric()
    assert DataType.date32().byte_width == 4
    ```

=== "Node"

    ```js
    const { DataType } = require('yggdryl').types

    const dt = DataType.ts64()
    assert(dt.name === 'ts64' && dt.category === 'temporal')
    assert(dt.isTemporal() && !dt.isNumeric())
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::{DataType, DataTypeId};
    use yggdryl_core::io::fixed::PrimitiveType;

    assert!(DataTypeId::Ts64.is_temporal());
    assert_eq!(DataTypeId::Ts96.name(), "ts96");
    ```

## Flexible parsing & native interop

Beyond the strict ISO `FromStr`, `parse_str` (Rust) / the `date` · `time` · `timestamp` · `duration`
factories (bindings) accept common formats — ISO, US `MM/DD/YYYY`, European `DD.MM.YYYY`, month
names (`Feb 29, 2024`), 12-hour times (`1:45 PM`), a date only, a time only, and a trailing zone —
with optional `unit` / `tz` for defaulting and casting while parsing. A **duration** parses a single
`<count><unit>` (`90s`), a compound run (`1h30m15s`, `2d 3h`, `1 hour 30 minutes`), a clock
(`1:30:00`), or ISO-8601 (`PT1H30M`, `P1DT2H`, `P2W`) — keeping the input's natural granularity, with
an optional `unit` to cast. The bindings also convert to/from the platform's native temporal types.

=== "Python"

    ```python
    import datetime
    from yggdryl.temporal import date, timestamp, Ts64, Duration64

    assert date("02/29/2024").to_ymd() == (2024, 2, 29)          # US format
    assert timestamp("2024-02-29 13:45:30", unit="ms").unit == "ms"  # default + cast

    # Native datetime interop.
    ts = Ts64.from_pydatetime(datetime.datetime(2024, 2, 29, 13, 45, 30))
    assert isinstance(ts.to_pydatetime(), datetime.datetime)
    assert Duration64.milliseconds(1500).to_timedelta() == datetime.timedelta(seconds=1.5)
    assert repr(ts) == "ts64[us](2024-02-29T13:45:30.000000)"   # signature + ISO value
    ```

=== "Node"

    ```js
    const { date, timestamp, Ts64 } = require('yggdryl').temporal

    assert.deepEqual(date('02/29/2024').toYmd(), [2024, 2, 29])   // US format
    assert.equal(timestamp('2024-02-29 13:45:30', 'ms').unit, 'ms')

    // JS Date bridge (a Date is milliseconds since the epoch).
    const ts = Ts64.fromDatetime(2024, 2, 29, 13, 45, 30, 0, 'ms', 'UTC')
    const js = new Date(ts.toEpochMillis())
    assert.equal(js.toISOString(), '2024-02-29T13:45:30.000Z')
    assert.equal(ts.signature(), 'ts64[ms, UTC](2024-02-29T13:45:30.000Z)')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::temporal::{Date32, Ts64, TimeUnit, Tz};

    assert_eq!(Date32::parse_str("Feb 29, 2024").unwrap().to_ymd(), (2024, 2, 29));
    let ts = Ts64::parse_str("2024-02-29 13:45:30", TimeUnit::Millisecond, Tz::UTC).unwrap();
    assert_eq!(format!("{ts:?}"), "ts64[ms, UTC](2024-02-29T13:45:30.000Z)"); // signature
    ```

## Arrow interop

The columnar temporal types (`Ts64Serie`, `Date32Serie`, …) convert to and from Arrow's
`Timestamp` / `Date32` / `Date64` / `Time32` / `Time64` / `Duration` arrays, carrying their
`(unit, tz)` — including the lossy `Ts32`/`Duration32` widen-to-`i64` and `Ts96` →
`FixedSizeBinary(12)` cases, and the Python zero-copy pyarrow bridge. See
[Arrow interop → Temporal](../arrow/temporal.md) for the full three-language reference.

## Design notes

- **Full IANA timezones, DST-correct.** `Tz` wraps `chrono` + `chrono-tz` (the compiled IANA
  database) behind this crate's own trait — those types never appear in a public signature. The
  offset lookup is a binary search in the database and is **allocation-free** (see the
  [benchmark report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/temporal.md)).
- **Exact calendar math.** Dates use Howard Hinnant's `days_from_civil` / `civil_from_days`,
  branchless and valid across the whole range; `to_datetime` for a naive/UTC instant is stack-only.
- **Value identity.** Each type is `Eq`/`Ord`(by instant/span)/`Hash` and has a byte codec, so it
  works as a map key and pickles / serializes. Resolving a *local* wall-clock to a UTC instant for
  an IANA zone uses a one-shot offset lookup — exact except within the ~1 hour/year around a DST
  transition (use a fixed offset when an exact boundary matters); the reverse (`to_datetime`) is
  always exact.
