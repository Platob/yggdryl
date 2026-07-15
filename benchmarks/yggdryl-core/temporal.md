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
cargo bench -p yggdryl-core --bench temporal
cargo test  -p yggdryl-core --test io_temporal
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
