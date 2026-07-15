# Arrow interop — temporal

The nine columnar temporal types (see [Types → Temporal](../types/temporal.md)) each map to an Arrow
array. This page walks them **simple → complex**, in synced three-language tabs. Two conventions run
throughout the bindings:

- a **cell** crosses as the value's **ISO-8601 string** (`get` / the constructor / `push`), or as its
  raw **epoch / physical count** (`get_epoch` → Python `int` / Node `bigint`; `from_epochs`);
- a column fixes one `(unit, tz)`; a value at another resolution is re-expressed at the column's unit.

| yggdryl column | Arrow array | unit param | zero-copy | lossy |
| --- | --- | --- | --- | --- |
| `Date32Serie`  | `Date32`  (i32 days)   | — (fixed Day)   | yes | no |
| `Date64Serie`  | `Date64`  (i64 millis) | — (fixed Milli) | yes | no |
| `Time32Serie`  | `Time32(Second\|Millisecond)`     | s / ms          | yes | no |
| `Time64Serie`  | `Time64(Microsecond\|Nanosecond)` | us / ns         | yes | no |
| `Ts64Serie`    | `Timestamp(unit, tz)`  | s / ms / us / ns | yes | no |
| `Duration64Serie` | `Duration(unit)`    | s / ms / us / ns | yes | no |
| `Ts32Serie`    | `Timestamp(unit, tz)` (**widened to i64**)  | s / ms / us / ns | no  | narrow width in metadata |
| `Duration32Serie` | `Duration(unit)` (**widened to i64**)    | s / ms / us / ns | no  | narrow width in metadata |
| `Ts96Serie`    | `FixedSizeBinary(12)` (**opaque**) | s / ms / us / ns | no | unit/tz in metadata |

Arrow can only model the four sub-second/second resolutions (`s` / `ms` / `us` / `ns`). The coarse
and calendar units (`min` … `y`) live only in yggdryl — see [the unit limit](#the-non-arrow-unit-limit).

Across the languages: **Rust** owns the `to_arrow_array` / `from_arrow_array` conversions; **Python**
exports and imports zero-copy through `pyarrow` (the Arrow C Data Interface); **Node** has no
Arrow-array bridge (apache-arrow JS ships no C Data Interface consumer), so its cross-language interop
is the shared `serializeBytes` wire form.

## 1. Dates — the simple case

`Date32` (days) and `Date64` (millis) carry **no unit parameter** — the resolution is fixed and the
zone is always naive. They map to Arrow's `Date32` / `Date64`.

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.temporal import Date32Serie

    col = Date32Serie("d", "naive", ["2024-07-15", None])   # cells are ISO date strings
    assert col.get(0) == "2024-07-15" and col.get_epoch(0) == 19_919   # days since the epoch

    arr = pa.array(col)                                     # zero-copy -> pyarrow date32
    assert arr.type == pa.date32()
    assert Date32Serie.from_arrow(arr) == col               # and back
    ```

=== "Node"

    ```js
    const { Date32Serie } = require('yggdryl').temporal

    const col = new Date32Serie('d', 'naive', ['2024-07-15', null])
    assert(col.get(0) === '2024-07-15')
    assert(col.getEpoch(0) === 19919n)                      // days since the epoch, as a bigint
    // Node interops through the byte codec (no Arrow-array bridge).
    assert(Date32Serie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    # #[cfg(feature = "arrow")]
    # fn demo() {
    use yggdryl_core::io::fixed::Date32Serie;
    use yggdryl_core::io::fixed::temporal::{Date32, TimeUnit, Tz};

    // Date32 fixes Day / naive.
    let col = Date32Serie::from_values(TimeUnit::Day, Tz::NAIVE, &[Date32::from_ymd(2024, 7, 15).unwrap()]).unwrap();
    let array = col.to_arrow_array().unwrap();              // zero-copy Date32Array
    let field = col.to_field("d").to_arrow();
    let back = Date32Serie::from_arrow_array(array.as_ref(), &field).unwrap();
    assert_eq!(back, col);
    # }
    ```

## 2. Times — a wall-clock, two widths

`Time32` takes `s` or `ms`; `Time64` takes `us` or `ns` (Arrow's exact split). Both are naive and map
to Arrow's `Time32(unit)` / `Time64(unit)`. The ISO cell shows the fractional digits of its unit
(`"13:45:30"` at `s`, `"13:45:30.000000"` at `us`).

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.temporal import Time64Serie

    col = Time64Serie("us", "naive", ["13:45:30.000000", None])
    arr = pa.array(col)
    assert arr.type == pa.time64("us")
    assert Time64Serie.from_arrow(arr) == col
    ```

=== "Node"

    ```js
    const { Time64Serie } = require('yggdryl').temporal

    const col = new Time64Serie('us', 'naive', ['13:45:30.000000', null])
    assert(col.get(0) === '13:45:30.000000')
    assert(Time64Serie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    # #[cfg(feature = "arrow")]
    # fn demo() {
    use yggdryl_core::io::fixed::Time64Serie;
    use yggdryl_core::io::fixed::temporal::{Time64, TimeUnit, Tz};

    let t = Time64::from_hms_nano(13, 45, 30, 0).unwrap().to_unit(TimeUnit::Microsecond).unwrap();
    let col = Time64Serie::from_values(TimeUnit::Microsecond, Tz::NAIVE, &[t]).unwrap();
    let array = col.to_arrow_array().unwrap();              // Time64Array(Microsecond)
    let field = col.to_field("t").to_arrow();
    assert_eq!(Time64Serie::from_arrow_array(array.as_ref(), &field).unwrap(), col);
    # }
    ```

## 3. Timestamps — unit **and** timezone

`Ts64` is a UTC-relative count at one of the four resolutions plus a timezone, mapping to Arrow's
`Timestamp(unit, tz)`. The zone crosses as a string — **naive** (`""`), **UTC**, a **fixed offset**
(`"+02:00"`), or a **DST-aware IANA** zone (`"Europe/Paris"`). The stored instant is UTC-relative, so
re-zoning never changes the physical count — only the wall-clock reading.

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.temporal import Ts64Serie

    # Cells are ISO-8601 strings; from_epochs takes raw counts in the column's unit.
    col = Ts64Serie("s", "UTC", ["2023-11-14T22:13:20Z", None])
    assert col.get(0) == "2023-11-14T22:13:20Z"
    assert col.get_epoch(0) == 1_700_000_000
    assert Ts64Serie.from_epochs("s", "UTC", [1_700_000_000]).get_epoch(0) == 1_700_000_000

    arr = pa.array(col)                                     # zero-copy
    assert arr.type == pa.timestamp("s", tz="UTC")
    assert Ts64Serie.from_arrow(arr) == col

    naive = Ts64Serie("ms", "", ["2023-11-14T22:13:20.000"])       # tz "" -> naive
    paris = Ts64Serie("us", "Europe/Paris", ["2024-07-15T14:00:00+02:00"])   # DST-aware IANA
    off   = Ts64Serie("ns", "+02:00", ["2024-07-15T14:00:00+02:00"])         # fixed offset
    ```

=== "Node"

    ```js
    const { Ts64Serie } = require('yggdryl').temporal

    const col = new Ts64Serie('s', 'UTC', ['2023-11-14T22:13:20Z', null])
    assert(col.get(0) === '2023-11-14T22:13:20Z')
    assert(col.getEpoch(0) === 1700000000n)                // bigint
    assert(Ts64Serie.fromEpochs('s', 'UTC', [1700000000n]).getEpoch(0) === 1700000000n)

    // Round-trip across languages via the shared wire form.
    assert(Ts64Serie.deserializeBytes(col.serializeBytes()).equals(col))

    const paris = new Ts64Serie('us', 'Europe/Paris', ['2024-07-15T14:00:00+02:00'])  // IANA
    const off   = new Ts64Serie('ns', '+02:00', ['2024-07-15T14:00:00+02:00'])        // offset
    ```

=== "Rust"

    ```rust
    # #[cfg(feature = "arrow")]
    # fn demo() {
    use yggdryl_core::io::fixed::Ts64Serie;
    use yggdryl_core::io::fixed::temporal::{Ts64, TimeUnit, Tz};

    let col = Ts64Serie::from_options(
        TimeUnit::Second,
        Tz::UTC,
        &[Some(Ts64::from_epoch(1_700_000_000, TimeUnit::Second, Tz::UTC).unwrap()), None],
    ).unwrap();

    let array = col.to_arrow_array().unwrap();             // TimestampSecondArray, tz = "UTC"
    let field = col.to_field("t").to_arrow();
    let back = Ts64Serie::from_arrow_array(array.as_ref(), &field).unwrap();
    assert_eq!(back, col);
    # }
    ```

## 4. Durations — an elapsed span

`Duration64` is a signed count at one of the four resolutions (no zone), mapping to Arrow's
`Duration(unit)`.

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.temporal import Duration64Serie

    col = Duration64Serie.from_epochs("ms", "", [1500, None])   # 1.5 s, and a null
    arr = pa.array(col)
    assert arr.type == pa.duration("ms")
    assert Duration64Serie.from_arrow(arr) == col
    ```

=== "Node"

    ```js
    const { Duration64Serie } = require('yggdryl').temporal

    const col = Duration64Serie.fromEpochs('ms', '', [1500n])
    assert(col.getEpoch(0) === 1500n)
    assert(Duration64Serie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    # #[cfg(feature = "arrow")]
    # fn demo() {
    use yggdryl_core::io::fixed::Duration64Serie;
    use yggdryl_core::io::fixed::temporal::{Duration64, TimeUnit, Tz};

    let col = Duration64Serie::from_values(TimeUnit::Millisecond, Tz::NAIVE, &[Duration64::milliseconds(1500)]).unwrap();
    let array = col.to_arrow_array().unwrap();             // DurationMillisecondArray
    let field = col.to_field("d").to_arrow();
    assert_eq!(Duration64Serie::from_arrow_array(array.as_ref(), &field).unwrap(), col);
    # }
    ```

## 5. Lossy — `Ts32` / `Duration32` widen to i64

Arrow has **no 32-bit** timestamp or duration, so `Ts32Serie` / `Duration32Serie` convert to a
**real** Arrow `Timestamp` / `Duration` (i64-backed) — the physical count widens losslessly. The
narrow **logical** width (`ts32` / `duration32`) is what would be lost, so `to_field(...).to_arrow`
records it under `yggdryl.logical_type`, and `from_arrow_array` recovers the 32-bit column from that
metadata (see [Metadata & round-tripping](metadata.md)).

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.temporal import Ts32Serie

    col = Ts32Serie("s", "UTC", ["2023-11-14T22:13:20Z"])
    arr = pa.array(col)                                    # a real timestamp('s', tz='UTC')
    assert arr.type == pa.timestamp("s", tz="UTC")
    assert Ts32Serie.from_arrow(arr) == col                # narrow width recovered from metadata
    ```

=== "Node"

    ```js
    const { Ts32Serie } = require('yggdryl').temporal

    const col = new Ts32Serie('s', 'UTC', ['2023-11-14T22:13:20Z'])
    assert(Ts32Serie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    # #[cfg(feature = "arrow")]
    # fn demo() {
    use yggdryl_core::io::fixed::Ts32Serie;
    use yggdryl_core::io::fixed::temporal::{Ts32, TimeUnit, Tz};

    let col = Ts32Serie::from_values(TimeUnit::Second, Tz::UTC, &[Ts32::from_epoch(1_700_000_000, TimeUnit::Second, Tz::UTC).unwrap()]).unwrap();
    let array = col.to_arrow_array().unwrap();             // i64-backed TimestampSecondArray
    let field = col.to_field("t").to_arrow();              // carries yggdryl.logical_type = "ts32"
    assert_eq!(Ts32Serie::from_arrow_array(array.as_ref(), &field).unwrap(), col);
    # }
    ```

## 6. Lossy — `Ts96` is an opaque `FixedSizeBinary(12)`

`Ts96` is a 96-bit (12-byte) count reaching far past `i64`'s ~292-year range (the year 5000+ at
nanosecond resolution). Arrow cannot model it, so `Ts96Serie` maps to an **opaque**
`FixedSizeBinary(12)` — the bytes round-trip, but the schema tag carries neither the temporal
meaning nor the `(unit, tz)`. Those are recovered from the field's `unit` / `timezone` metadata.

=== "Python"

    ```python
    import pyarrow as pa
    from yggdryl.temporal import Ts96Serie

    col = Ts96Serie("ns", "UTC", ["2023-11-14T22:13:20Z"])
    arr = pa.array(col)                                    # fixed_size_binary(12), opaque
    assert arr.type == pa.binary(12)
    assert Ts96Serie.from_arrow(arr) == col                # (unit, tz) recovered from metadata
    ```

=== "Node"

    ```js
    const { Ts96Serie } = require('yggdryl').temporal

    const col = new Ts96Serie('ns', 'UTC', ['2023-11-14T22:13:20Z'])
    assert(Ts96Serie.deserializeBytes(col.serializeBytes()).equals(col))
    ```

=== "Rust"

    ```rust
    # #[cfg(feature = "arrow")]
    # fn demo() {
    use yggdryl_core::io::fixed::Ts96Serie;
    use yggdryl_core::io::fixed::temporal::{Ts96, TimeUnit, Tz};

    let col = Ts96Serie::from_values(TimeUnit::Nanosecond, Tz::UTC, &[Ts96::from_epoch(1_700_000_000_000_000_000, TimeUnit::Nanosecond, Tz::UTC).unwrap()]).unwrap();
    let array = col.to_arrow_array().unwrap();             // FixedSizeBinary(12)
    let field = col.to_field("t").to_arrow();              // unit + timezone in metadata
    assert_eq!(Ts96Serie::from_arrow_array(array.as_ref(), &field).unwrap(), col);
    # }
    ```

## The metadata round-trip

Every temporal field records its `(unit, tz)` under the reserved `unit` / `timezone` keys, and the
lossy widths add `yggdryl.logical_type` — so a naive `ts32` at second resolution, a `ts96`, or an
`Europe/Paris` timestamp all reconstruct exactly, even across an Arrow IPC/Parquet round-trip that
carries the unknown keys through. See [Metadata & round-tripping](metadata.md) for the key table.

## The non-Arrow-unit limit

Arrow models only `s` / `ms` / `us` / `ns`. A timestamp, time, or duration column at a **coarse or
calendar** unit (`min`, `h`, `w`, `mo`, `y` — anything `Minute` … `Year`) has no Arrow form, so
`to_arrow_array` returns a **guided error** naming the unit; the column still serializes and
round-trips through the byte codec. (Dates are unaffected — `Date32`/`Date64` fix Day/Milli and map
to Arrow's dedicated `Date32`/`Date64`.)

```rust
# #[cfg(feature = "arrow")]
# fn demo() {
use yggdryl_core::io::fixed::Ts64Serie;
use yggdryl_core::io::fixed::temporal::{Ts64, TimeUnit, Tz};

let minutes = Ts64Serie::from_values(TimeUnit::Minute, Tz::UTC, &[Ts64::from_epoch(28_333_333, TimeUnit::Minute, Tz::UTC).unwrap()]).unwrap();
assert!(minutes.to_arrow_array().is_err());   // Minute is not an Arrow unit — guided error
# }
```
