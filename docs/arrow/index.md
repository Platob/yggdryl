# Arrow interop — the invariants

Apache Arrow is yggdryl's **physical layer** (`arrow-buffer`), so every value/column type has a
defined relationship to the Arrow ecosystem. This section gathers those rules in one place, then
drills down per type family:

| page | covers |
| --- | --- |
| [Primitives](primitives.md) | native zero-copy ints/floats + wide-int / `FixedSizeBinary` closest-fit |
| [Decimals](decimal.md) | `Decimal{32,64,128,256}Array` zero-copy |
| [Temporal](temporal.md) | all 9 date/time/timestamp/duration types, unit + timezone, lossy widen cases |
| [Nested](nested.md) | `StructArray` / `RecordBatch` + the Python PyCapsule bridge |
| [Metadata & round-tripping](metadata.md) | the reserved keys that make a lossy schema round-trip |

## Five invariants

**1. `to_arrow` is total — closest-fit fallback.** A type's `to_arrow()` / `arrow_data_type()`
always returns *some* `arrow_schema::DataType`: the **exact** primitive when Arrow has one
(`i32` → `Int32`), else the **closest optimized representation** — `Decimal128(38,0)` /
`Decimal256(76,0)` for wide signed integers (a scale-0 decimal is an integer), `FixedSizeBinary(N)`
for a width Arrow cannot model (`u128`, `u96`/`i96`, `u256`, fixed-size utf8), `Float16` for `f16`.
Zero-copy is a *capability*, never a requirement — the schema mapping never fails.

**2. `ArrowNative` is the zero-copy capability.** Sharing an allocation with an
`arrow_array::PrimitiveArray` (an `Arc` bump, never a payload copy) is gated on the `ArrowNative`
sub-trait, implemented only for the types with a real `ArrowPrimitiveType` (`u8`…`i64`,
`f16`/`f32`/`f64`, and the decimal coefficients). A type without it — `u96`, `i96`, `u256`,
fixed-size utf8 — is still a first-class value (full codec, `Buffer`/`Serie`, serialization); it
just lacks the shared-`Arc` array round-trip. The validity bitmap is LSB-first with `1 = valid`,
byte-identical to Arrow's `NullBuffer`, so nulls round-trip too.

**3. `yggdryl.logical_type` recovers lossy, non-injective mappings.** Because `to_arrow` is lossy
and *non-injective* (`u96`, `i96`, `FixedUtf8`, and a runtime-`N` `FixedBinary` all collapse to the
same `FixedSizeBinary(N)`), a naive `from_arrow` would have to *guess*. Instead the exact logical
type is recorded in field metadata under a reserved key — **only when the plain mapping is
ambiguous** — and `from_arrow` prefers it to recover the precise type, falling back to the safe base
(`FixedSizeBinary` → `fixed_binary`, never a guessed wide integer) when it is absent. See
[Metadata & round-tripping](metadata.md).

**4. The `arrow` feature scopes it all.** The array/schema conversions live behind the core's
**`arrow`** Cargo feature — the crate still builds (full codec, columns, serialization) without it.
Arrow's data types are the physical layer; the closest-fit mapping is centralized once on
`DataTypeId::to_arrow` / `from_arrow`, so the erased and typed descriptors share it.

**5. Arrow types never appear in a non-`arrow`-gated public signature.** `arrow-rs` types are an
implementation detail: they surface only in `#[cfg(feature = "arrow")]` methods
(`to_arrow_array`, `to_arrow`, `from_arrow_array`, …). The rest of the API — and every binding —
speaks only in yggdryl types.

## Across the three languages

- **Rust** is where the Arrow conversions live: `Buffer` / `Serie` / `DecimalSerie` /
  `TemporalSerie` ↔ `arrow_array::*Array`, and `DataType` / `Field` ↔ `arrow_schema::*`.
- **Python** additionally bridges the **nested** and **temporal** columns to `pyarrow` **zero-copy**
  through the Arrow **C Data Interface** (the PyCapsule protocol, `__arrow_c_array__` /
  `__arrow_c_schema__`), so `pyarrow.array(col)` exports with no payload copy and
  `from_arrow(...)` imports back. The numeric and decimal columns interop through the byte codec.
- **Node** has no Arrow-array bridge (apache-arrow JS ships no C Data Interface consumer), so
  cross-language interop is the shared `serializeBytes` / `deserializeBytes` wire form — the exact
  same bytes the Rust core reads to build its Arrow arrays.
