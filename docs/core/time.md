# Calendar & time

The standard libraries have no civil date/time types, so yggdryl ships its own —
`Date`, `Time`, `DateTime`, `Duration` and a DST-aware `Timezone` — with **no
external timezone database**. The same surface in Python, Node and Rust; every type
parses/renders a canonical string, converts to/from a component map and bytes, and
is hashable and serializable.

## Date

A proleptic-Gregorian calendar date (days since the UNIX epoch). It validates the
calendar, orders chronologically, and exposes the components.

=== "Python"

    ```python
    import yggdryl

    d = yggdryl.Date(2024, 2, 29)          # a leap day
    assert (d.year, d.month, d.day) == (2024, 2, 29)
    assert str(d) == "2024-02-29"
    assert d.weekday == 4                   # Thursday (0 = Sunday)
    assert d.add_days(1) == yggdryl.Date(2024, 3, 1)
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const d = new yggdryl.Date(2024, 2, 29);
    // [d.year, d.month, d.day] === [2024, 2, 29]
    d.toString();              // "2024-02-29"
    d.addDays(1).toString();   // "2024-03-01"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Date;

    let d = Date::from_ymd(2024, 2, 29)?;
    assert_eq!((d.year(), d.month(), d.day()), (2024, 2, 29));
    assert_eq!(d.to_str(), "2024-02-29");
    ```

## Time

A time of day with nanosecond resolution.

=== "Python"

    ```python
    import yggdryl

    t = yggdryl.Time(13, 45, 30, 250_000_000)
    assert str(t) == "13:45:30.250"
    assert yggdryl.Time.from_str("13:45:30.250") == t
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const t = new yggdryl.Time(13, 45, 30, 250000000);
    t.toString(); // "13:45:30.250"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Time;

    let t = Time::from_hms_nano(13, 45, 30, 250_000_000)?;
    assert_eq!(t.to_str(), "13:45:30.250");
    ```

## Timezone & DST

A `Timezone` is `UTC`, a fixed `±HH:MM` offset, or a named IANA zone. Named zones
carry an **embedded POSIX-TZ rule**, so the standard-vs-DST offset for any instant is
computed with no external tz database.

=== "Python"

    ```python
    import yggdryl

    ny = yggdryl.Timezone("America/New_York")
    # January is EST (UTC-5); July is EDT (UTC-4).
    assert ny.offset_seconds(1_704_067_200) == -5 * 3600
    assert ny.offset_seconds(1_719_792_000) == -4 * 3600
    assert yggdryl.Timezone("+05:30").offset_seconds(0) == 19_800
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const ny = new yggdryl.Timezone("America/New_York");
    ny.offsetSeconds(1704067200); // -18000 (EST)
    ny.offsetSeconds(1719792000); // -14400 (EDT)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Timezone;

    let ny = Timezone::from_str("America/New_York")?;
    assert_eq!(ny.offset_seconds(1_704_067_200), -5 * 3600);
    assert_eq!(ny.offset_seconds(1_719_792_000), -4 * 3600);
    ```

!!! note
    Coverage is UTC, every fixed offset, raw POSIX TZ strings and a curated table of
    common IANA zone names. DST uses each zone's **current** rule — historical
    transitions (which need the full tz database) are not modelled.

## DateTime

An absolute instant (UTC seconds + nanoseconds) with an optional display
`Timezone`. Conversions are a pure display change — the underlying instant never
moves — and are DST-aware.

=== "Python"

    ```python
    import yggdryl

    utc = yggdryl.DateTime.from_str("2024-07-01T12:00:00Z")
    assert utc.epoch_seconds == 1_719_835_200
    # The same instant displayed in New York (EDT) and Tokyo.
    ny = utc.to_timezone("America/New_York")
    assert (ny.hour, str(ny)) == (8, "2024-07-01T08:00:00-04:00")
    assert utc.to_timezone("Asia/Tokyo").hour == 21
    # Resolve a wall-clock time in a zone back to its instant.
    local = yggdryl.DateTime(2024, 7, 1, 8, 0, 0, 0, "America/New_York")
    assert local.epoch_seconds == utc.epoch_seconds
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const utc = yggdryl.DateTime.fromStr("2024-07-01T12:00:00Z");
    utc.epochSeconds;                       // 1719835200
    const ny = utc.toTimezone("America/New_York");
    ny.hour;                                // 8
    ny.toString();                          // "2024-07-01T08:00:00-04:00"
    utc.epochNanos;                         // 1719835200000000000n (BigInt)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{DateTime, Timezone};

    let utc = DateTime::from_str("2024-07-01T12:00:00Z")?;
    assert_eq!(utc.epoch_seconds(), 1_719_835_200);
    let ny = utc.to_timezone(Timezone::from_str("America/New_York")?);
    assert_eq!(ny.hour(), 8);
    ```

## Duration

A signed span of time with nanosecond resolution, parsed from a compact form.

=== "Python"

    ```python
    import yggdryl

    d = yggdryl.Duration.from_str("1h30m")
    assert d.as_seconds() == 5_400
    assert (d + yggdryl.Duration.from_secs(30)).as_seconds() == 5_430
    assert yggdryl.Duration.from_unit(500, "ms").as_nanos() == 500_000_000
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const d = yggdryl.Duration.fromStr("1h30m");
    d.asSeconds();                          // 5400
    yggdryl.Duration.fromUnit(500, "ms").asNanos(); // 500000000n (BigInt)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Duration, TimeUnit};

    let d = Duration::from_str("1h30m")?;
    assert_eq!(d.as_seconds(), 5_400);
    assert_eq!(Duration::from_unit(500, TimeUnit::Millisecond).as_nanos(), 500_000_000);
    ```

## Conversion & flexible parsing

The point-in-time types share a `Temporal` interface and convert into one another:
a `Date` becomes a midnight `DateTime` (`to_datetime`), a `DateTime` yields its
local `Date` / `Time`, and a `Date` can be `at(time)` to a zoned instant. Parsing is
flexible — `Date` accepts `2024/07/01`, `20240701` or a full datetime; `DateTime`
accepts a date-only string (→ midnight) or a bare integer (→ epoch seconds);
`Duration` accepts ISO-8601 (`PT15M`, `P1D`) as well as the compact form. `from_str`
is the single parsing entry point and **raises** on malformed input (there is no
lenient mode).

=== "Python"

    ```python
    import yggdryl

    assert str(yggdryl.Date.from_str("2024/07/01")) == "2024-07-01"  # flexible format
    assert yggdryl.Duration.from_str("PT15M").as_seconds() == 900
    d = yggdryl.Date(2024, 7, 1).with_timezone("America/New_York")
    assert d.at(yggdryl.Time(8, 0, 0)).epoch_seconds == 1_719_835_200
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    yggdryl.Date.fromStr("20240701").toString();      // "2024-07-01" (flexible format)
    yggdryl.Duration.fromStr("PT15M").asSeconds();    // 900
    new yggdryl.Date(2024, 7, 1).withTimezone("America/New_York")
      .at(new yggdryl.Time(8, 0, 0)).epochSeconds;    // 1719835200
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Date, Duration, Temporal, Time, Timezone};

    assert!(Date::from_str("not-a-date").is_err());
    assert_eq!(Duration::from_str("PT15M")?.as_seconds(), 900);
    let d = Date::from_ymd(2024, 7, 1)?.with_timezone(Timezone::from_str("America/New_York")?);
    assert_eq!(d.at(Time::from_hms(8, 0, 0)?).epoch_seconds(), 1_719_835_200);
    ```

## Next

- [DataType](../schema/datatype.md) — the schema layer's temporal types reuse this
  `TimeUnit` and `Timezone`.
- Back to [Getting started](../getting-started.md)
