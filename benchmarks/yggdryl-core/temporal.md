# `io::fixed::temporal` — benchmark & optimization notes

Time **and** memory for the temporal value types: the civil-calendar math (`Date` ↔ `(y, m, d)`),
the timestamp wall-clock decomposition (naive/UTC vs a DST-aware IANA zone, which consults the tz
database), unit conversion, and the byte codec. The point is to show the calendar/decompose paths
are **stack-only**, and that even the IANA-zone step — the one place a DST offset is looked up in
the full tz database (via `chrono-tz`) — stays **allocation-free**. Dependency-free harness (~1 s)
with a counting global allocator; the deterministic `io_temporal` tests assert the correctness
(leap years, DST transitions, ISO parsing) these numbers ride on.

## Run

```bash
cargo bench -p yggdryl-core --bench temporal                     # value types only
cargo bench -p yggdryl-core --features arrow --bench temporal    # + columnar Arrow interop
cargo test  -p yggdryl-core --test io_temporal
cargo test  -p yggdryl-core --features arrow --test temporal_arrow --test temporal_alloc
```

## Rust core (release, counting global allocator)

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `Date32::from_ymd` (calendar math) | (inlined) | **0.00** | 0.0 |
| `Date32::to_ymd` (calendar math) | (inlined) | **0.00** | 0.0 |
| `Ts64::to_datetime` (UTC) | ~20 | **0.00** | 0.0 |
| `Ts64::to_datetime` (IANA, DST lookup) | ~9 | **0.00** | 0.0 |
| `Ts64::from_datetime` (UTC) | ~350 | **0.00** | 0.0 |
| `Ts64::to_unit` (s → ms) | ~380 | **0.00** | 0.0 |
| `Ts64` serialize+deserialize (zoned) | ~4 | 3.00 | 46.0 |
| `Duration64::checked_add` (unit align) | (inlined) | **0.00** | 0.0 |
| `Date32::at_time` → `Ts64` (combine) | ~24000 | **0.00** | 0.0 |
| `Ts64::to_date` (extract) | ~17 | **0.00** | 0.0 |
| `Ts64::to_duration` (span) | (inlined) | **0.00** | 0.0 |
| `Duration64::to_timestamp` | (inlined) | **0.00** | 0.0 |
| `Tz::offset_seconds_at` (IANA DST lookup) | ~43 | **0.00** | 0.0 |
| `Duration64::parse_str` (`"1h30m15s"`) | ~2 | 4.00 | 4.0 |

## What the numbers show

- **The calendar math is exact and free.** `Date32::from_ymd` / `to_ymd` use Howard Hinnant's
  `days_from_civil` / `civil_from_days` — branchless integer arithmetic, `0 allocs / op`, valid for
  the whole `i64` day range. `from_datetime` / `to_unit` are likewise stack-only (`~350–380 Mops`).
- **The UTC/naive decomposition is stack-only.** `to_datetime` for a naive or UTC instant splits
  epoch nanoseconds into a day + time-of-day and runs the civil conversion — `0 allocs / op`.
- **The IANA DST lookup is ~2× the UTC path but still allocation-free.** A zoned `to_datetime`
  binary-searches the compiled IANA tz database (`chrono-tz`) for the offset in effect at the
  instant — `~9 Mops` vs `~20` for UTC, and crucially **`0 allocs / op`** (the search is on the
  stack). So DST-correct wall-clock reads cost roughly one extra table lookup, not an allocation.
- **Only the string-carrying codec allocates.** `serialize_bytes` writes the count + unit tag +
  the timezone *name*, so a zoned round-trip is `3 allocs` (the byte `Vec`, the name `String`, and
  the parse on the way back). The value types themselves are `Copy` and allocation-free everywhere
  else.
- **The cross-type converters are free.** `Date::at_time` / `Ts::to_date` / `Ts::to_duration` /
  `Duration::to_timestamp` — the full "any temporal → any temporal" matrix — are stack-only integer
  arithmetic over the shared epoch-nanosecond axis, `0 allocs / op`. A converter that consults a
  zone (through an instant) rides the same allocation-free IANA lookup.
- **The one expensive step, isolated, is still allocation-free.** `Tz::offset_seconds_at` — the DST
  offset lookup the zoned paths depend on — is `~43 Mops`, `0 allocs / op`: a binary search on the
  stack, no heap traffic.
- **The flexible duration parser allocates only for lowercasing.** `Duration64::parse_str`
  (compound / clock / ISO-8601) is `~4 allocs` — one per unit token, from `TimeUnit::parse`'s
  case-fold; the bare-`m` fast path avoids a redundant one. Parsing isn't a hot loop, so this is
  left as-is rather than trading it for a no-alloc case-insensitive matcher.

## Columnar Arrow interop (feature `arrow`)

The `TemporalSerie<B>` ↔ Arrow columnar path. Each `op` is one operation over a whole
**4096-element** column, so `Mops/s` counts column-ops (× 4096 for elements/s); the story is the
`bytes/op` column — the raw counts are a `4096 × width` payload (32 KiB for `ts64`), and the native
export **and** import must never copy it.

| op | Mops/s | allocs/op | bytes/op |
|----|-------:|----------:|---------:|
| `Ts64Serie::from_options` (build) | 0.06 | 9 | 33840 |
| `Ts64Serie::to_arrow_array` (native, share) | 0.92 | 7 | **755** |
| `Ts64Serie::from_arrow_array` (share payload) | 0.02 | 3 | **1048** |
| `Ts64Serie::serialize_bytes` | 0.07 | 21 | 67984 |
| `Ts64Serie::deserialize_bytes` | 0.01 | 10 | 100485 |
| `Ts32Serie::to_arrow_array` (widen i32→i64) | 0.16 | 7 | 33011 |
| `Ts32Serie::from_arrow_array` (narrow) | 0.03 | 3 | 16464 |
| `Ts96Serie::to_arrow_array` (FSB12, share) | 1.55 | 3 | **176** |
| `Ts96Serie::from_arrow_array` (FSB12, share) | 0.02 | 2 | **34** |

### What the columnar numbers show

- **The native `ts64` export shares its 32 KiB payload.** `to_arrow_array` allocates only the
  4096-bit (512-byte) null bitmap + Arrow shells — `755 bytes/op`, **flat in the payload**. The
  counts `Arc` is bumped, not copied.
- **The `ts64` import is zero-copy on the fast path (the optimization).** `from_arrow_array` over a
  dense/offset-0/no-garbage array (every yggdryl-produced array) shares the values `Arc` and only
  rebuilds the null mask — `1048 bytes/op` (the validity bitmap + framework), **not** the 32 KiB
  payload. Before the optimization it always copied `len*width` and rebuilt every cell; the
  `temporal_alloc` test pins a **dense** (null-free) import at `24 B/op` vs a garbage-under-null
  slow-path copy at `≥ 32768 B/op`.
- **The `ts32` widen pays exactly one buffer.** Arrow has no 32-bit temporal type, so `ts32` /
  `duration32` sign-extend into one fresh `i64` buffer on export (`33011 bytes/op ≈ 32 KiB`) and
  narrow back into an `i32` buffer on import (`16464 ≈ 16 KiB`) — the unavoidable width change, one
  allocation, no more.
- **`ts96` byte data shares too.** The `FixedSizeBinary(12)` form needs no element alignment, so both
  directions are a pure `Arc` bump — `176` / `34 bytes/op`, independent of the 48 KiB of counts.
- **The `serialize_bytes` codec is a full copy by design** (a self-describing frame, not zero-copy):
  the larger `bytes/op` there is the owned `Vec` + sink, unrelated to the Arrow path.
